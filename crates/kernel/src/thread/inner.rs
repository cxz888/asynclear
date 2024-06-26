use crate::{signal::KSignalSet, trap::TrapContext};

pub struct ThreadInner {
    /// 陷入上下文
    pub trap_context: TrapContext,

    // TODO: [blocked] thread。实现 clear_child_tid。<https://man7.org/linux/man-pages/man2/set_tid_address.2.html>
    #[allow(unused)]
    pub clear_child_tid: usize,

    // 信号
    /// 信号掩码
    pub signal_mask: KSignalSet,
    /// 待处理信号队列
    pub pending_signal: KSignalSet,
}
