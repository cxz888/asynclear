use core::sync::atomic::AtomicUsize;

use atomic::Ordering;
use bitflags::bitflags;
use common::config::{PAGE_OFFSET_MASK, PAGE_SIZE, PAGE_SIZE_BITS};
use compact_str::CompactString;
use defines::{error::KResult, misc::TimeSpec};
use delegate::delegate;
use futures::future::BoxFuture;
use klocks::{RwLock, SpinMutex};
use triomphe::Arc;

use super::{
    dentry::DEntryDir,
    page_cache::{BackedPage, PageCache},
};
use crate::{executor::block_on, fs::page_cache::PageState, memory::Frame, time::curr_time};

static INODE_NUMBER: AtomicUsize = AtomicUsize::new(0);

pub type DynDirInode = Inode<dyn DirInodeBackend>;
pub type DynPagedInode = Inode<PagedInode<dyn PagedInodeBackend>>;
pub type DynStreamInode = Inode<dyn StreamInodeBackend>;

pub struct Inode<T: ?Sized> {
    meta: InodeMeta,
    pub inner: T,
}

impl<T> Inode<T> {
    pub fn new(meta: InodeMeta, inner: T) -> Self {
        Self { meta, inner }
    }
}

impl<T: ?Sized> Inode<T> {
    pub fn meta(&self) -> &InodeMeta {
        &self.meta
    }
}

pub struct InodeMeta {
    /// inode number，在一个文件系统中唯一标识一个 Inode
    ino: usize,
    mode: StatMode,
    name: CompactString,
    inner: SpinMutex<InodeMetaInner>,
}

impl InodeMeta {
    pub fn new(mode: StatMode, name: CompactString) -> Self {
        Self {
            ino: INODE_NUMBER.fetch_add(1, Ordering::SeqCst),
            mode,
            name,
            inner: SpinMutex::new(InodeMetaInner {
                access_time: TimeSpec::default(),
                modify_time: TimeSpec::default(),
                change_time: TimeSpec::default(),
            }),
        }
    }

    pub fn ino(&self) -> usize {
        self.ino
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn mode(&self) -> StatMode {
        self.mode
    }

    pub fn lock_inner_with<T>(&self, f: impl FnOnce(&mut InodeMetaInner) -> T) -> T {
        f(&mut self.inner.lock())
    }
}

pub struct InodeMetaInner {
    /// 上一次访问时间
    pub access_time: TimeSpec,
    /// 上一次修改时间
    pub modify_time: TimeSpec,
    /// 上一次元数据变化时间
    pub change_time: TimeSpec,
}

pub trait DirInodeBackend: Send + Sync {
    fn lookup(&self, name: &str) -> Option<DynInode>;
    /// 调用者保证一定是目录类型，且传入的 `mode` 也是 [`StatMode::S_IFDIR`]
    fn mkdir(&self, name: CompactString, mode: StatMode) -> KResult<Arc<DynDirInode>>;
    fn read_dir(&self, parent: &Arc<DEntryDir>) -> KResult<()>;
}

impl<T: ?Sized + DirInodeBackend> Inode<T> {
    delegate! {
        to self.inner {
            pub fn lookup(&self, name: &str) -> Option<DynInode>;
            pub fn mkdir(&self, name: CompactString, mode: StatMode) -> KResult<Arc<DynDirInode>>;
            pub fn read_dir(&self, dentry: &Arc<DEntryDir>) -> KResult<()>;
        }
    }
}

/// 可以按页级别进行读写的 inode，一般应该是块设备做后备
pub struct PagedInode<T: ?Sized> {
    data_len: RwLock<usize>,
    page_cache: RwLock<PageCache>,
    backend: T,
}

impl<T> PagedInode<T> {
    pub fn new(backend: T, data_len: usize) -> Self {
        Self {
            data_len: RwLock::new(data_len),
            page_cache: RwLock::new(PageCache::new()),
            backend,
        }
    }
}

impl<T: ?Sized> PagedInode<T> {
    pub fn data_len(&self) -> usize {
        *self.data_len.read()
    }
}

pub trait PagedInodeBackend: Send + Sync {
    fn read_page(&self, frame: &mut Frame, page_id: usize) -> KResult<()>;
    fn write_page(&self, frame: &Frame, page_id: usize) -> KResult<()>;
}

impl<T: ?Sized + PagedInodeBackend> PagedInode<T> {
    pub fn read_at(&self, meta: &InodeMeta, buf: &mut [u8], offset: usize) -> KResult<usize> {
        let data_len = *self.data_len.read();

        if offset >= data_len {
            return Ok(0);
        }

        let read_end = usize::min(buf.len(), data_len - offset);
        let mut nread = 0;

        while nread < read_end {
            let page_id = (offset + nread) >> PAGE_SIZE_BITS;
            let page_offset = (offset + nread) & PAGE_OFFSET_MASK;
            let page = self.get_or_init_page(page_id);

            // 检查页状态，如有必要则读后备文件
            if page.state.load(Ordering::SeqCst) == PageState::Invalid {
                let mut _guard = block_on(page.state_guard.lock());
                if page.state.load(Ordering::SeqCst) == PageState::Invalid {
                    self.backend
                        .read_page(&mut *page.inner.frame_mut(), page_id)?;
                    page.state.store(PageState::Synced, Ordering::SeqCst);
                }
            }
            let frame = page.inner.frame();

            let copy_len = usize::min(read_end - nread, PAGE_SIZE - page_offset);
            buf[nread..nread + copy_len]
                .copy_from_slice(&frame.as_page_bytes()[page_offset..page_offset + copy_len]);
            nread += copy_len;
        }
        meta.lock_inner_with(|inner| inner.access_time = TimeSpec::from(curr_time()));

        Ok(nread)
    }

    pub fn write_at(&self, meta: &InodeMeta, buf: &[u8], offset: usize) -> KResult<usize> {
        let curr_data_len = *self.data_len.read();
        let curr_last_page_id = curr_data_len >> PAGE_SIZE_BITS;

        // 写范围是 offset..offset + buf.len()。
        // 中间可能有一些页被完全覆盖，因此可以直接设为 Dirty 而不需要读
        let full_page_range =
            (offset & !PAGE_OFFSET_MASK)..(offset + buf.len()).next_multiple_of(PAGE_SIZE);

        let mut nwrite = 0;

        while nwrite < buf.len() {
            let page_id = (offset + nwrite) >> PAGE_SIZE_BITS;
            let page_offset = (offset + nwrite) & PAGE_OFFSET_MASK;
            let page = self.get_or_init_page(page_id);

            let mut frame;
            if page.state.load(Ordering::SeqCst) == PageState::Invalid {
                let mut _guard = block_on(page.state_guard.lock());
                frame = page.inner.frame_mut();
                if page_id <= curr_last_page_id
                    && full_page_range.contains(&page_id)
                    && page.state.load(Ordering::SeqCst) == PageState::Invalid
                {
                    self.backend.read_page(&mut *frame, page_id)?;
                }
                page.state.store(PageState::Dirty, Ordering::SeqCst);
            } else {
                frame = page.inner.frame_mut();
            }

            let copy_len = usize::min(buf.len() - nwrite, PAGE_SIZE - page_offset);
            frame.as_page_bytes_mut()[page_offset..page_offset + copy_len]
                .copy_from_slice(&buf[nwrite..nwrite + copy_len]);
            nwrite += copy_len;
        }
        meta.lock_inner_with(|inner| {
            inner.access_time = TimeSpec::from(curr_time());
            inner.change_time = inner.access_time;
            inner.modify_time = inner.change_time;
        });
        if curr_data_len < offset + buf.len() {}

        Ok(nwrite)
    }

    fn get_or_init_page(&self, page_id: usize) -> Arc<BackedPage> {
        let page = self.page_cache.read().get(page_id);
        page.unwrap_or_else(|| self.page_cache.write().create(page_id))
    }
}

pub trait StreamInodeBackend: Send + Sync {
    fn read(&self, meta: &InodeMeta, buf: &mut [u8]) -> BoxFuture<'_, KResult<usize>>;
    fn write(&self, meta: &InodeMeta, buf: &[u8]) -> BoxFuture<'_, KResult<usize>>;
}

pub enum DynInode {
    Dir(Arc<DynDirInode>),
    Paged(Arc<DynPagedInode>),
    Stream(Arc<DynStreamInode>),
}

impl DynInode {
    pub fn meta(&self) -> &InodeMeta {
        match self {
            DynInode::Dir(dir) => dir.meta(),
            DynInode::Paged(paged) => paged.meta(),
            DynInode::Stream(stream) => stream.meta(),
        }
    }
}

bitflags! {
    /// The mode of a inode
    /// whether a directory or a file
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct StatMode: u32 {
        // 以下类型只为其一
        /// 是普通文件
        const REGULAR       = 1 << 15;
        /// 是符号链接
        const SYM_LINK      = 1 << 15 | 1 << 13;
        /// 是 socket
        const SOCKET        = 1 << 15 | 1 << 14;
        /// 是块设备
        const BLOCK_DEVICE  = 1 << 14 | 1 << 13;
        /// 是目录
        const DIR           = 1 << 14;
        /// 是字符设备
        const CHAR_DEVICE   = 1 << 13;
        /// 是 FIFO
        const FIFO          = 1 << 12;

        /// 是否设置 uid/gid/sticky
        // const S_ISUID = 1 << 11;
        // const S_ISGID = 1 << 10;
        // const S_ISVTX = 1 << 9;
        // TODO: 由于暂时没有权限系统，目前全设为 777
        /// 所有者权限
        const S_IRWXU = Self::S_IRUSR.bits() | Self::S_IWUSR.bits() | Self::S_IXUSR.bits();
        const S_IRUSR = 1 << 8;
        const S_IWUSR = 1 << 7;
        const S_IXUSR = 1 << 6;
        /// 用户组权限
        const S_IRWXG = Self::S_IRGRP.bits() | Self::S_IWGRP.bits() | Self::S_IXGRP.bits();
        const S_IRGRP = 1 << 5;
        const S_IWGRP = 1 << 4;
        const S_IXGRP = 1 << 3;
        /// 其他用户权限
        const S_IRWXO = Self::S_IROTH.bits() | Self::S_IWOTH.bits() | Self::S_IXOTH.bits();
        const S_IROTH = 1 << 2;
        const S_IWOTH = 1 << 1;
        const S_IXOTH = 1 << 0;
    }
}

pub macro DynDirInodeCoercion() {
    #[allow(unused_unsafe)]
    unsafe {
        ::unsize::Coercion::new({
            #[allow(unused_parens)]
            fn coerce<'lt>(
                p: *const Inode<impl DirInodeBackend + 'lt>,
            ) -> *const Inode<dyn DirInodeBackend + 'lt> {
                p
            }
            coerce
        })
    }
}

pub macro DynPagedInodeCoercion() {
    #[allow(unused_unsafe)]
    unsafe {
        ::unsize::Coercion::new({
            #[allow(unused_parens)]
            fn coerce<'lt>(
                p: *const Inode<PagedInode<impl PagedInodeBackend + 'lt>>,
            ) -> *const Inode<PagedInode<dyn PagedInodeBackend + 'lt>> {
                p
            }
            coerce
        })
    }
}
