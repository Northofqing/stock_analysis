//! Opening inventory is not a strategy entry or a fabricated trade.
use super::*;
use crate::performance::economic_position::{
    CostBasisKind, EconomicFillRow, FillCostEvidence, FillCostLedger,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpeningInventoryExit {
    pub fill_id: i64,
    pub original_quantity: u32,
    pub opening_quantity: u32,
    pub strategy_quantity: u32,
    pub seed_lot_ids: Vec<String>,
    pub opening_basis: Money,
    pub opening_buy_fee: Money,
    pub opening_sell_fee: Money,
    pub opening_net_pnl: Money,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectiveExitPnl {
    pub fill_id: i64,
    pub date: NaiveDate,
    pub account_net_pnl: Money,
    /// None when this exit consumes only opening inventory.
    pub strategy_net_pnl: Option<Money>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OpeningInventorySample {
    pub projection_hash: String,
    pub as_of: NaiveDate,
    pub seed_binding: Option<AccountBinding>,
    pub remaining_opening_lots: Vec<Lot>,
    pub remaining_lots: Vec<Lot>,
    pub excluded_exits: Vec<OpeningInventoryExit>,
    pub exit_pnls: Vec<EffectiveExitPnl>,
    #[serde(skip_serializing)]
    pub(crate) strategy_rows: Vec<EconomicFillRow>,
    #[serde(skip_serializing)]
    pub(crate) strategy_costs: FillCostLedger,
}
#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveSellablePosition {
    pub code: String,
    pub name: String,
    pub quantity: u32,
    pub basis: Money,
    pub buy_fee: Money,
    pub first_buy_date: NaiveDate,
}
impl OpeningInventorySample {
    pub fn sellable_positions(&self) -> Result<Vec<EffectiveSellablePosition>, LedgerError> {
        let mut groups = BTreeMap::<String, EffectiveSellablePosition>::new();
        for lot in &self.remaining_lots {
            if lot.sellable_from > self.as_of {
                continue;
            }
            let group =
                groups
                    .entry(lot.code.clone())
                    .or_insert_with(|| EffectiveSellablePosition {
                        code: lot.code.clone(),
                        name: lot.name.clone(),
                        quantity: 0,
                        basis: Money::ZERO,
                        buy_fee: Money::ZERO,
                        first_buy_date: lot.acquired_on,
                    });
            group.quantity = group
                .quantity
                .checked_add(lot.quantity)
                .ok_or(LedgerError::Overflow)?;
            group.basis = group.basis.add(lot.basis_price.mul(lot.quantity)?)?;
            group.buy_fee = group.buy_fee.add(lot.buy_fee_remaining)?;
            group.first_buy_date = group.first_buy_date.min(lot.acquired_on);
        }
        Ok(groups.into_values().collect())
    }
}
fn portion(amount: Money, part: u32, total: u32) -> Result<Money, LedgerError> {
    if total == 0 || part > total {
        return Err(LedgerError::IntegrityFailure(
            "invalid FIFO allocation".into(),
        ));
    }
    Ok(Money(
        i64::try_from(i128::from(amount.0) * i128::from(part) / i128::from(total))
            .map_err(|_| LedgerError::Overflow)?,
    ))
}
impl VerifiedEffectiveFillSet {
    pub fn opening_inventory_sample(&self) -> Result<OpeningInventorySample, LedgerError> {
        self.require_economic_price_authority()?;
        self.opening_inventory_sample_for_diagnostics()
    }

    // Keep complete FIFO facts for diagnostic economic lifecycle reconstruction;
    // this sample must not escape as qualified account or settlement amounts.
    pub(crate) fn opening_inventory_sample_for_diagnostics(
        &self,
    ) -> Result<OpeningInventorySample, LedgerError> {
        let mut lots = self
            .seed_lots
            .iter()
            .cloned()
            .map(|lot| (lot, true))
            .collect::<Vec<_>>();
        let mut rows = Vec::new();
        let mut costs = Vec::new();
        let mut exclusions = Vec::new();
        let mut exit_pnls = Vec::new();
        for original in self.rows()? {
            let mut row = original.clone();
            let quantity = u32::try_from(row.quantity).map_err(|_| LedgerError::Overflow)?;
            let price = Money::from_cny(row.fill_price.ok_or_else(|| {
                LedgerError::IntegrityFailure("effective fill price absent".into())
            })?)?;
            let at = crate::trading::paper_lot_ledger::parse_paper_fill_timestamp(
                row.id,
                &row.occurred_at,
            )
            .map_err(LedgerError::IntegrityFailure)?
            .date();
            let mut row_fee = adjudication::fee(price, quantity, &row.direction)?;
            match row.direction.as_str() {
                "buy" => lots.push((
                    Lot {
                        lot_id: format!("effective:{}", row.id),
                        code: row.code.clone(),
                        name: row.name.clone(),
                        quantity,
                        basis_price: price,
                        buy_fee_remaining: row_fee,
                        acquired_on: at,
                        sellable_from: crate::calendar::verified_next_a_share_trading_day(at)
                            .map_err(LedgerError::EvidenceUnavailable)?,
                        reported_cost: None,
                    },
                    false,
                )),
                "sell" => {
                    let mut left = quantity;
                    let mut opening = 0u32;
                    let mut opening_basis = Money::ZERO;
                    let mut opening_buy_fee = Money::ZERO;
                    let mut basis = Money::ZERO;
                    let mut buy_fee = Money::ZERO;
                    let mut seed_lot_ids = Vec::new();
                    for (lot, is_opening) in &mut lots {
                        if left == 0 || lot.code != row.code || lot.sellable_from > at {
                            continue;
                        }
                        let taken = left.min(lot.quantity);
                        let allocated = portion(lot.buy_fee_remaining, taken, lot.quantity)?;
                        let consumed_basis = lot.basis_price.mul(taken)?;
                        basis = basis.add(consumed_basis)?;
                        buy_fee = buy_fee.add(allocated)?;
                        if *is_opening {
                            opening = opening.checked_add(taken).ok_or(LedgerError::Overflow)?;
                            opening_basis = opening_basis.add(consumed_basis)?;
                            opening_buy_fee = opening_buy_fee.add(allocated)?;
                            seed_lot_ids.push(lot.lot_id.clone());
                        }
                        lot.buy_fee_remaining = lot.buy_fee_remaining.sub(allocated)?;
                        lot.quantity -= taken;
                        left -= taken;
                    }
                    if left != 0 {
                        return Err(LedgerError::EvidenceUnavailable(format!(
                            "effective FIFO/T+1 exit {} lacks {left} shares",
                            row.id
                        )));
                    }
                    lots.retain(|(lot, _)| lot.quantity > 0);
                    let opening_sell_fee = portion(row_fee, opening, quantity)?;
                    let opening_net = price
                        .mul(opening)?
                        .sub(opening_basis)?
                        .sub(opening_buy_fee)?
                        .sub(opening_sell_fee)?;
                    let account_net = price
                        .mul(quantity)?
                        .sub(basis)?
                        .sub(buy_fee)?
                        .sub(row_fee)?;
                    let strategy_quantity = quantity - opening;
                    exit_pnls.push(EffectiveExitPnl {
                        fill_id: row.id,
                        date: at,
                        account_net_pnl: account_net,
                        strategy_net_pnl: (strategy_quantity > 0)
                            .then_some(account_net.sub(opening_net)?),
                    });
                    if opening > 0 {
                        exclusions.push(OpeningInventoryExit {
                            fill_id: row.id,
                            original_quantity: quantity,
                            opening_quantity: opening,
                            strategy_quantity,
                            seed_lot_ids,
                            opening_basis,
                            opening_buy_fee,
                            opening_sell_fee,
                            opening_net_pnl: opening_net,
                        });
                    }
                    row.quantity = i64::from(strategy_quantity);
                    row_fee = row_fee.sub(opening_sell_fee)?;
                    if strategy_quantity == 0 {
                        continue;
                    }
                }
                _ => {
                    return Err(LedgerError::IntegrityFailure(
                        "invalid effective side".into(),
                    ))
                }
            }
            costs.push(FillCostEvidence {
                fill_id: row.id,
                adverse_cost: row_fee.cny(),
                evidence_id: format!(
                    "OpeningInventorySampleV1:{}:{}:{}",
                    self.receipt.projection_hash, row.id, row.quantity
                ),
            });
            rows.push(row);
        }
        let seed_binding = match &self.receipt.request.scope {
            EffectiveFillScope::Epoch(binding) => Some(binding.clone()),
            _ => None,
        };
        Ok(OpeningInventorySample {
            projection_hash: self.receipt.projection_hash.clone(),
            as_of: self.receipt.request.as_of,
            seed_binding,
            remaining_opening_lots: lots
                .iter()
                .filter(|(_, opening)| *opening)
                .map(|(lot, _)| lot.clone())
                .collect(),
            remaining_lots: lots.into_iter().map(|(lot, _)| lot).collect(),
            excluded_exits: exclusions,
            exit_pnls,
            strategy_rows: rows,
            strategy_costs: FillCostLedger {
                basis_id: format!(
                    "OpeningInventorySampleV1:{FEE_MODEL}:{}",
                    self.receipt.projection_hash
                ),
                kind: CostBasisKind::Scenario,
                costs,
            },
        })
    }
}
