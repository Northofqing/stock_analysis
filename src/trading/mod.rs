//! 交易执行抽象层（M2-1）。
//!
//! 目标：在不接入真实券商前，先统一模拟交易与未来实盘交易的接口边界。
//! 当前提供 SimulatedExecutionGateway（落库到 stock_position）。

use crate::database::DatabaseManager;
use crate::models::{NewStockPosition, StockPosition};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    Accepted,
    Filled,
    Rejected,
    Canceled,
}

#[derive(Debug, Clone)]
pub struct OrderReceipt {
    pub business_order_id: String,
    pub side: OrderSide,
    pub status: OrderStatus,
    pub code: String,
    pub quantity: i32,
    pub price: f64,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct OpenPositionCmd {
    pub business_order_id: String,
    pub code: String,
    pub name: String,
    pub trade_date: String,
    pub price: f64,
    pub quantity: i32,
}

#[derive(Debug, Clone)]
pub struct ClosePositionCmd {
    pub business_order_id: String,
    pub position_id: i32,
    pub code: String,
    pub trade_date: String,
    pub price: f64,
    pub quantity: i32,
}

#[derive(Debug, Clone)]
pub struct CancelOrderCmd {
    pub business_order_id: String,
    pub code: String,
}

pub trait TradeExecutionGateway {
    fn get_open_position(&self, code: &str) -> Result<Option<StockPosition>, String>;
    fn update_position_return(&self, id: i32, current_price: f64, return_rate: f64) -> Result<(), String>;
    fn open_position(&self, cmd: &OpenPositionCmd) -> Result<OrderReceipt, String>;
    fn close_position(&self, cmd: &ClosePositionCmd) -> Result<OrderReceipt, String>;
    fn cancel_order(&self, cmd: &CancelOrderCmd) -> Result<OrderReceipt, String>;
}

pub struct SimulatedExecutionGateway;

impl SimulatedExecutionGateway {
    pub fn new() -> Self {
        Self
    }

    fn db(&self) -> &'static DatabaseManager {
        DatabaseManager::get()
    }
}

impl TradeExecutionGateway for SimulatedExecutionGateway {
    fn get_open_position(&self, code: &str) -> Result<Option<StockPosition>, String> {
        self.db().get_open_position(code).map_err(|e| e.to_string())
    }

    fn update_position_return(&self, id: i32, current_price: f64, return_rate: f64) -> Result<(), String> {
        self.db()
            .update_position_return(id, current_price, return_rate)
            .map_err(|e| e.to_string())
    }

    fn open_position(&self, cmd: &OpenPositionCmd) -> Result<OrderReceipt, String> {
        let new_position = NewStockPosition {
            code: cmd.code.clone(),
            name: cmd.name.clone(),
            buy_date: cmd.trade_date.clone(),
            buy_price: cmd.price,
            quantity: cmd.quantity,
            status: "open".to_string(),
        };

        self.db()
            .save_position(&new_position)
            .map_err(|e| e.to_string())?;

        Ok(OrderReceipt {
            business_order_id: cmd.business_order_id.clone(),
            side: OrderSide::Buy,
            status: OrderStatus::Filled,
            code: cmd.code.clone(),
            quantity: cmd.quantity,
            price: cmd.price,
            message: "simulated open position filled".to_string(),
        })
    }

    fn close_position(&self, cmd: &ClosePositionCmd) -> Result<OrderReceipt, String> {
        self.db()
            .close_position(cmd.position_id, cmd.price, &cmd.trade_date)
            .map_err(|e| e.to_string())?;

        Ok(OrderReceipt {
            business_order_id: cmd.business_order_id.clone(),
            side: OrderSide::Sell,
            status: OrderStatus::Filled,
            code: cmd.code.clone(),
            quantity: cmd.quantity,
            price: cmd.price,
            message: "simulated close position filled".to_string(),
        })
    }

    fn cancel_order(&self, cmd: &CancelOrderCmd) -> Result<OrderReceipt, String> {
        Ok(OrderReceipt {
            business_order_id: cmd.business_order_id.clone(),
            side: OrderSide::Buy,
            status: OrderStatus::Rejected,
            code: cmd.code.clone(),
            quantity: 0,
            price: 0.0,
            message: "simulated gateway does not support pending-order cancel".to_string(),
        })
    }
}
