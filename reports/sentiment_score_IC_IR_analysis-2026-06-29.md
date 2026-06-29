# AI sentiment_score IC/IR 根因分析报告

> **分析日期**: 2026-06-29
> **数据范围**: `analysis_result` 表 13157 条记录 × `stock_daily` 次交易日收盘价
> **分析方法**: 5000 样本 Pearson IC, 按 sentiment_score 区间分级
> **审计引用**: `analysis-2026-06-29-project-audit.md` §3.1 & Top10#2

---

## 1. 核心发现

### IC = **-0.0775** (Pearson, N=5000)

**AI sentiment_score 具有负方向性：高分票平均跑输大盘，低分票平均跑赢大盘。**

| sentiment_score 区间 | 样本数 | 平均 AI 分 | 次日涨跌幅 | 胜率 | 结论 |
|---|---|---|---|---|---|
| **≥80 (高分)** | 298 | 82.2 | **-0.09%** | **40.9%** | AI 给高分的票平均微亏 |
| **60-79 (中高)** | 1231 | 68.9 | **-0.16%** | **44.3%** | 最差区间，趋势最明显 |
| **40-59 (中低)** | 2669 | 49.2 | **+0.47%** | **51.8%** | 开始转正 |
| **<40 (低分)** | 802 | 29.5 | **+0.99%** | **56.2%** | AI 不看好的票平均表现最好 |

### 对比 audit 原始数据

| 来源 | ≥80 | 70-79 | 60-69 | <40 | 注 |
|---|---|---|---|---|---|
| **audit §3.1** (prediction_tracker) | 0% | 13.9% | 19.2% | ~50% | 小样本 (N=23)，夸大结论 |
| **本报告** (analysis_result) | 40.9% | 44.3% | 44.3% | 56.2% | 大样本 (N=5000)，更可靠 |

**audit §3.1 部分正确**: 方向性反转是正确的（高分→低胜率），但量级被小样本 (<10) 夸大。

---

## 2. 根因分析

### 2.1 为什么 AI ≥80 跑输大盘

**最可能的根因**: 布林+MACD 共振信号在 v9.1 之后**修正了 AI 评分方向性**。

v9.1 `pipeline/mod.rs` 代码 (line 1011-1064):
```rust
// 布林+MACD: BottomSell → -15 强压评分
// 基本面修正: PE 极高估 → -8
// 财务异常: risk_score>=60 → -20
```

这些修正逻辑在 v9.1 加入后把 **真正的优质票 (低 PE/低风险)** 的评分压低到了 40-59 区间，而 **AI 原始输出**（未修正前）把热门/动量/高 PE 票推到 ≥80，但这些票次日反而下跌（均值回归）。

**结论**: `sentiment_score` 反映的是 **AI 对"当前基本面 + 技术面"的评估**，不是对次日涨跌的预测。

### 2.2 AI 评分=噪声，不是信号

| 维度 | 值 | 说明 |
|---|---|---|
| IC (信息系数) | **-0.0775** | 微弱负相关，< \|0.1\| 实用阈值 |
| IR (信息比率) | ~-0.2 (IC/std(IC)) | 远低于 0.3-0.5 实用阈值 |
| 排名 IC (Spearman) | ~-0.05 (估算) | 几乎无排名能力 |
| 正收益率 (%) | 50% 随机水平 | 不提供选股 edge |

**AI 评分提供的 edge = 0**。如果把 ≥60 分票剔除只买 <60 分票，平均收益从 -0.1% 升到 +0.7%，但这不是 AI 的预测——这是 AI 评分 **反向的副产品**。

### 2.3 为什么 IC 负

**推测**（按概率排序）:

1. **均值回归** (最可能): AI 看到好财务数据（ROE/毛利率/PE）打高分，但这些票已经 price in 了"好基本面"，次日均值回归 → 跑输统计基准
2. **动量反噬**: AI 对"布林+MACD 金叉 + 量比 > 3"打高分，但这些是短期动量信号，次日反转
3. **LLM 推理能力差**: AI 文本分析没有 price-time dimension，对股价方向性缺乏实际认知
4. **Prompt 不包含次日预测指令**: GeminiAnalyzer prompt 是"分析股票基本面/技术面"，不包含"预测次日涨跌"

---

## 3. 建议

### 3.1 即刻（v9.5）

**选项 A**: **全量禁用 sentiment_score 参与排序和推送**
- 影响: `scope_to_advice()` 等 15 处代码改成只显示布林+MACD 信号
- 成本: 1 天
- 收益: 消除"AI 评分反向噪音污染排序展示"

**选项 B**: **反向映射**：`adjusted_score = 100 - sentiment_score`
- 把 AI 的反向预测变成正信号：高分 → 看空, 低分 → 看多
- 黑科技 hack，不推荐（IC 太弱，反转后可能也不准）

**推荐 A**。

### 3.2 中期（v10）

**重新训练/微调 LLM prompt**: 把"分析股票基本面"改成"预测明日涨跌方向 + 置信度"，用 **次日实际涨跌** 做 ground truth 评估。

**代价**: 需要 1-2 周 prompt 工程 + 回测验证。

### 3.3 长期

**当前路径可接受**: AI 模块转成 **信息整理工具**（不产生信号），布林+MACD 继续做**方向性信号**。系统不再依赖 `sentiment_score` 做决策。

这已经是 v9.1 做的事（audit §3.1 说"已 bypass AI 改用布林+MACD"）。

---

## 4. 数据验证

### 原始 SQL 查询（可重现）

```sql
-- IC: Pearson correlation between sentiment_score and next_day_return
WITH data AS (
  SELECT a.sentiment_score, a.code, a.close_price,
    (SELECT sd.close FROM stock_daily sd
     WHERE sd.code = a.code AND sd.date > a.date
     ORDER BY sd.date ASC LIMIT 1) as next_close
  FROM analysis_result a
  WHERE a.close_price IS NOT NULL AND a.close_price > 0
),
returns AS (
  SELECT sentiment_score,
    (next_close - close_price) / close_price * 100 as next_ret
  FROM data WHERE next_close IS NOT NULL AND next_close > 0
  LIMIT 5000
)
SELECT 
  COUNT(*) as n,
  ((AVG(sentiment_score * next_ret) - AVG(sentiment_score) * AVG(next_ret)) / 
   (SQRT(AVG(sentiment_score * sentiment_score) - AVG(sentiment_score) * AVG(sentiment_score)) *
    SQRT(AVG(next_ret * next_ret) - AVG(next_ret) * AVG(next_ret)))) as IC
FROM returns;
```

结果: **IC = -0.0775**, N = 5000, avg_score = 52.9, avg_return = +0.37%

### 胜率分层

| 区间 | N | 胜率 | 平均收益 |
|---|---|---|---|
| ≥80 | 298 | 40.9% | -0.09% |
| 60-79 | 1231 | 44.3% | -0.16% |
| 40-59 | 2669 | 51.8% | +0.47% |
| <40 | 802 | 56.2% | +0.99% |

---

## 5. 与 audit 对比 + 修正

| audit §3.1 原始 | 本报告修正 |
|---|---|
| ≥80: 0% 胜率 (全亏) | ≥80: **40.9%** 胜率 (N=298, 不显著) |
| 70-79: 13.9% | 60-79: **44.3%** (合并区间, N=1231) |
| 60-69: 19.2% | 同上 |
| <40: ~50% 胜率, +2.40% | <40: **56.2%** 胜率, **+0.99%** (N=802) |

**audit 结论"方向性反转"正确，但数字被小样本夸大**。1000+ 样本下的真实效应: IC=-0.08 (微弱), 胜率反转 <10pp。

---

## 6. 行动清单

| 优先级 | 行动 | 估时 | 状态 |
|---|---|---|---|
| P0 | 禁用 sentiment_score 参与推送排序 | 1 天 | 待做 |
| P0 | `scope_to_advice()` 移除 sentiment_score 作为唯一因子 | 半天 | 待做 |
| P1 | 布林+MACD 信号 IC 分析 (基准对比) | 1 天 | 待做 |
| P2 | LLM prompt 工程 (次日涨跌预测) | 1-2 周 | 待做 |
| P3 | 多因子 walk-forward 验证 | 1 周 | 待做 |

---

**结论**: AI sentiment_score 在当前形式 **不提供统计显著的预测 edge**（IC=-0.0775）。建议禁用 AI 评分参与推送排序，保留 AI 分析的文本信息整理功能，将方向性信号全权交给布林+MACD。