#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Market {
    Shanghai,
    Shenzhen,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarCategory {
    Day,
    Minute,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adjustment {
    None,
    Forward,
    Backward,
}
