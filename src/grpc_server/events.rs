//! TDX 异动事件生成器 (合同 §8: price/volume/amount/status/reset 事件;
//! cursor generation+sequence 单调递增; UNADMITTED 影子事件必须显式隔离)。
//! Task 11 实现 diff 检测器 + EventHub + MarketEventService (Subscribe/Replay/GetListenerStatus)。
