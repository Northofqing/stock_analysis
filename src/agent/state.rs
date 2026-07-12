#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    Thinking,  // 拆解任务，决定使用哪个工具
    Acting,    // 调用数据接口
    Observing, // 处理返回的数据，校验是否满足要求
    Reporting, // 生成最终 Markdown 报告
}
