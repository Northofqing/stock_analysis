//! 新闻 → 产业链映射。
//!
//! 关键词规则表（优先）+ AI 推理兜底（规则未命中时）+ 动态板块成份股解析。
//! 不写死标的，标的从 sector_monitor 实时拉。

use crate::market_analyzer::sector_monitor;

#[derive(Debug, Clone)]
pub struct StockInfo {
    pub code: String,
    pub name: String,
    /// 当日涨跌幅 (%)：用于低位卡位/追高风险判定
    pub change_pct: f64,
    /// 量比：>1 表示今日放量，资金开始关注
    pub vol_ratio: f64,
}

/// 产业链命中来源，用于可观测性与降级标记。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChainSource {
    /// 关键词规则表命中
    #[default]
    Rule,
    /// 规则未命中，AI 推理产出
    Ai,
    /// 规则未命中且 AI 不可用（降级，不编造产业链）
    AiDegraded,
}

#[derive(Debug, Clone)]
pub struct ChainHit {
    pub chain: String,
    pub keywords: Vec<String>,
    pub logic: String,
    pub stocks: Vec<StockInfo>,
    /// 命中来源（规则 / AI / AI降级）
    pub source: ChainSource,
    /// 用于动态匹配板块代码的板块名关键词；AI 来源时由 AI 提供，规则来源可留空（从规则表查）
    pub board_keyword: String,
    /// 匹配板块的今日主力净占比(%)；None = 资金数据不可用（不臆测多空）
    pub fund_flow_pct: Option<f64>,
}

/// 关键词 → (关键词列表, 产业链名, 催化逻辑, 板块关键词, 优先级)
/// toml 不可用时的编译期 fallback。与 config/chain_rules.toml 保持同步。
const DEFAULT_CHAIN_RULES: &[(&[&str], &str, &str, &str, u32)] = &[
    // ── AI硬件 ──
    (&["液冷", "液冷服务器", "冷板", "浸没式冷却", "浸没式液冷", "冷却液", "液冷机柜", "液冷散热", "散热"], "AI硬件-液冷", "高功率密度算力集群驱动散热全面向液冷升级", "液冷", 100),
    (&["CPO", "光模块", "光通信", "光器件", "光互联", "硅光", "硅光子", "硅光模块", "光引擎", "共封装光学", "LPO", "1.6T光模块", "800G光模块"], "AI硬件-CPO", "AI算力驱动高速光互联需求，CPO/LPO/硅光渗透率快速提升", "CPO", 100),
    (&["铜缆", "高速连接", "DAC", "ACC", "AEC", "有源铜缆", "有源电缆", "高速线缆", "高速背板", "背板连接", "连接器", "铜连接", "铜互连", "高速互联", "内部互联"], "AI硬件-铜缆高速连接", "短距高速互连从光向铜延伸，GB300/GB200机柜内部铜缆方案催化", "铜缆高速连接", 100),
    (&["HBM", "HBM2", "HBM2E", "HBM3", "HBM3E", "HBM4", "高带宽内存", "堆叠内存", "3D堆叠内存", "混合键合", "内存接口", "内存接口芯片", "HBM封装"], "HBM-高带宽内存", "HBM3E量产+HBM4路线图+AI芯片拉动高带宽内存需求爆发", "HBM", 100),
    (&["PCB", "电路板", "覆铜板", "电子布", "印制电路", "印制电路板", "HDI板", "HDI", "IC载板", "ABF载板", "高频高速板", "服务器PCB", "封装基板"], "AI硬件-PCB", "电子布提价+AI服务器PCB需求激增，HDI与IC载板持续升级", "PCB", 90),
    (&["MLCC", "被动元件", "陶瓷电容", "片式电容", "薄膜电容", "电感", "电阻", "电容", "被动器件", "元器件"], "AI硬件-MLCC", "AI服务器MLCC用量从2000颗激增至数十万颗，被动元件持续受益", "MLCC", 90),
    (&["NVLink", "InfiniBand", "IB交换机", "NVSwitch", "Spectrum", "英伟达交换机", "英伟达互联", "GPU互联", "片间互联"], "AI硬件-NVLink", "英伟达NVLink/InfiniBand高速互联技术迭代，拉动配套硬件需求", "英伟达", 95),
    (&["AI服务器", "算力", "GPU", "数据中心", "智算中心", "算力租赁", "算力基建", "训练集群", "GPU集群", "超算", "东数西算", "算力调度", "国产算力"], "AI算力", "算力基建持续扩张，国产GPU+算力租赁+智算中心三线并进", "算力", 60),
    (&["Rubin", "Vera", "英伟达Rubin", "NVLink6", "Rubin Ultra", "Rubin平台", "英伟达下一代芯片"], "Rubin", "英伟达Rubin下一代AI芯片平台发布，拉动产业链备货需求", "英伟达", 95),
    (&["服务器电源", "AI电源", "高功率电源", "冗余电源", "电源模块", "UPS", "备用电源", "电源管理", "功率密度", "机柜电源"], "AI硬件-服务器电源", "AI服务器功耗激增，电源功率密度升级与冗余配置需求爆发", "算力", 90),
    // ── 半导体 ──
    (&["先进封装", "Chiplet", "CoWoS", "2.5D封装", "3D封装", "扇出型封装", "FOPLP", "面板级封装", "TSV", "玻璃基板", "玻璃通孔", "TGV", "混合键合", "晶圆级封装", "系统级封装", "SiP"], "半导体-先进封装", "Chiplet与HBM推动先进封装扩产，CoWoS/玻璃基板/PLP技术迭代", "先进封装", 95),
    (&["光刻机", "刻蚀机", "薄膜沉积", "PVD", "CVD", "ALD", "离子注入", "清洗设备", "CMP", "抛光机", "检测设备", "量测设备", "半导体设备", "晶圆厂", "半导体扩产"], "半导体-设备", "国产替代+晶圆厂扩产周期，半导体设备国产化率持续提升", "光刻机", 90),
    (&["光刻胶", "光刻胶树脂", "光刻胶溶剂", "光刻胶显影剂", "电子气体", "电子特气", "六氟化钨", "氟化氢", "电子化学品", "靶材", "抛光液", "抛光垫", "前驱体", "硅片", "大硅片", "ABF绝缘膜", "薄膜铌酸锂", "电子铜箔", "磷化铟"], "半导体-材料", "半导体材料国产替代加速，从光刻胶到电子气体的全链条突破", "光刻胶", 90),
    (&["存储芯片", "NAND", "NOR Flash", "DRAM", "内存芯片", "存储涨价", "存储器", "闪存", "内存颗粒", "利基型存储", "存储周期"], "半导体-存储芯片", "存储芯片涨价周期+国产NAND/DRAM突破，存储产业链价值重估", "存储芯片", 85),
    (&["晶圆代工", "Foundry", "成熟制程", "先进制程", "制程节点", "28nm", "14nm", "7nm", "5nm", "产能利用率", "流片", "代工"], "半导体-制造代工", "成熟制程产能爬坡+先进制程突破，代工格局重塑", "半导体", 80),
    (&["AI芯片", "NPU", "TPU", "ASIC芯片", "推理芯片", "训练芯片", "GPU替代", "国产GPU", "GPGPU", "智能芯片", "AI加速卡", "存算一体"], "半导体-AI芯片", "国产AI芯片在训练与推理两侧加速渗透，ASIC定制化趋势明确", "AI芯片", 90),
    (&["RISC-V", "RISC-V架构", "开源指令集", "开放架构", "RISC-V芯片", "RISC-V处理器", "RISC-V IP", "开源芯片", "指令集架构"], "半导体-RISC-V", "RISC-V开放架构在全球半导体博弈中的战略价值持续提升", "RISC-V", 85),
    (&["半导体", "芯片", "晶圆", "封测", "硅片", "国产替代", "集成电路", "IC设计", "IDM", "Fabless"], "半导体", "国产替代+扩产周期，半导体产业链整体受益", "半导体", 30),
    // ── AI应用与终端 ──
    (&["AI手机", "AIPC", "AI PC", "AI眼镜", "AI耳机", "端侧大模型", "端侧模型", "端云协同", "本地推理", "端侧推理", "端侧AI", "终端AI", "智能穿戴", "AI换机"], "AI终端", "端侧大模型落地带动AI手机/AIPC/AI眼镜换机周期", "AI手机", 85),
    (&["AI Agent", "AI智能体", "智能体", "MCP协议", "大模型应用", "行业大模型", "垂直大模型", "AI软件", "AI应用", "AI+", "人工智能应用", "GPT-5", "GPT-4o"], "AI应用", "AI Agent与行业大模型从概念走向落地，应用层百花齐放", "人工智能", 80),
    (&["智能驾驶", "自动驾驶", "无人驾驶", "L3自动驾驶", "L4自动驾驶", "端到端智驾", "智驾", "高阶智驾", "城市NOA", "高速NOA", "FSD", "FSD入华", "激光雷达", "LiDAR", "毫米波雷达", "智驾芯片", "域控制器", "智能座舱", "ADAS"], "智能驾驶", "L3级自动驾驶政策破冰+端到端大模型重塑智驾技术路线", "无人驾驶", 85),
    // ── 新能源 ──
    (&["固态电池", "全固态电池", "半固态电池", "硫化物固态", "氧化物固态", "固态电解质", "固态电池量产", "固态电池装车"], "新能源-固态电池", "固态电池技术路线收敛，硫化物路线与产业化进程加速", "固态电池", 95),
    (&["钠离子电池", "钠电池", "钠电", "层状氧化物", "聚阴离子", "普鲁士蓝", "钠离子储能", "硬碳", "钠离子量产"], "新能源-钠离子电池", "钠离子电池量产线投产，成本优势打开储能与两轮车替代空间", "钠离子电池", 90),
    (&["锂电池", "锂电", "磷酸铁锂", "LFP", "三元锂", "NCM", "电解液", "正极材料", "负极材料", "碳酸锂", "六氟磷酸锂", "锂矿", "盐湖提锂", "电池回收", "4680", "大圆柱电池", "刀片电池"], "新能源-锂电池", "锂电池技术迭代+成本下降+储能需求双轮驱动", "锂电池", 60),
    (&["光伏", "硅料", "组件", "逆变器", "钙钛矿", "钙钛矿叠层", "异质结", "HJT", "TOPCon", "BC电池", "光伏出海", "光伏装机", "分布式光伏"], "新能源-光伏", "钙钛矿叠层效率突破+光伏出海加速+行业供需再平衡", "光伏", 65),
    (&["氢能", "氢能源", "绿氢", "电解水制氢", "PEM电解槽", "碱性电解槽", "燃料电池", "氢燃料电池", "质子交换膜", "加氢站", "储氢", "氢能重卡", "氢储能"], "新能源-氢能", "氢能纳入国家能源体系，绿氢制备与燃料电池示范城市群加速", "氢能", 80),
    (&["储能", "电化学储能", "储能电站", "独立储能", "共享储能", "储能变流器", "PCS", "BMS", "EMS", "储能电池", "液流电池", "全钒液流", "压缩空气储能", "新型储能", "储能装机"], "新能源-储能", "新型储能装机持续超预期，独立储能商业模式逐步跑通", "储能", 75),
    // ── 电力电网 ──
    (&["变压器", "电力变压器", "配电变压器", "干式变压器", "特高压", "升压站", "换流站", "换流变压器", "电网改造", "配电网", "输变电", "变电站", "取向硅钢"], "电力-变压器", "算力扩容+电网升级+新能源并网三重共振，变压器进入长景气周期", "特高压", 85),
    (&["智能电网", "配电网", "电网数字化", "配网自动化", "电网改造", "变电站", "电力物联网", "电网智能化", "柔性直流", "继电保护", "配网设备", "环网柜"], "电力-智能电网", "新型电力系统建设加速，配网自动化与电网数字化投资高增", "智能电网", 80),
    (&["虚拟电厂", "需求响应", "负荷聚合", "辅助服务", "电力现货", "电力市场化", "电力交易", "售电", "可调负荷", "VPP", "分布式能源", "需求侧管理"], "电力-虚拟电厂", "电力市场化改革深化，需求侧响应与虚拟电厂聚合交易加速", "虚拟电厂", 85),
    (&["绿色电力", "绿电交易", "绿证", "新能源消纳", "可再生能源", "碳交易", "碳排放", "电力市场化", "CCER", "碳配额"], "电力-绿色电力", "绿电交易扩容+消纳机制完善+碳市场联动，绿电价值重估", "绿色电力", 75),
    // ── 机器人 ──
    (&["机器人", "人形机器人", "具身智能", "机械臂", "四足机器人", "服务机器人", "工业机器人", "协作机器人", "灵巧手", "机器人关节", "机器人电机", "机器人传感器", "机器人减速器", "宇树", "优必选", "Figure", "Tesla Bot", "Optimus", "机器人量产", "丝杠", "滚柱丝杠", "执行器"], "机器人", "人形机器人从实验室走向产线，具身智能+产业基金+量产预期驱动", "机器人", 80),
    // ── 高端制造 ──
    (&["低空经济", "无人机", "eVTOL", "飞行汽车", "低空空域", "通用航空", "通航", "城市空中交通", "UAM", "适航认证", "空域管理", "低空基础设施"], "低空经济", "低空空域管理改革+适航认证加速+eVTOL商业化运营在即", "低空经济", 85),
    (&["商业航天", "卫星互联网", "低轨卫星", "星链", "Starlink", "可回收火箭", "商业发射", "火箭发动机", "固体火箭", "卫星", "卫星通信", "遥感卫星", "北斗", "卫星制造", "地面终端", "相控阵天线"], "商业航天", "卫星互联网纳入新基建+可回收火箭突破+商业发射密度提升", "商业航天", 85),
    (&["量子计算", "量子芯片", "超导量子", "光量子", "离子阱", "量子比特", "量子纠错", "量子优越性", "量子通信", "量子密钥", "QKD", "量子网络", "量子精密测量"], "量子计算", "量子计算里程碑式突破，量子芯片+量子通信实用化加速", "量子计算", 80),
    (&["稀土", "稀土永磁", "永磁材料", "钕铁硼", "稀土磁材", "氧化镨钕", "镨钕", "镝", "铽", "重稀土", "轻稀土", "稀土配额", "稀土供给", "磁材"], "稀土永磁", "稀土供给管控+人形机器人/新能源车拉动永磁需求结构性增长", "稀土永磁", 80),
    // ── 政策驱动 ──
    (&["城市更新", "老旧小区", "地下管网", "棚改", "城中村", "旧城改造", "市政管网", "海绵城市", "城市基础设施", "燃气管道", "供水管道", "综合管廊"], "城市更新", "15万亿城市更新规划落地，管网改造与城中村项目密集开工", "地下管网", 70),
    (&["节能降碳", "碳达峰", "碳中和", "双碳", "减排", "节能改造", "钢铁改造", "水泥改造", "石化改造", "碳排放", "碳交易", "能耗双控", "超低排放", "碳捕集", "CCUS"], "节能降碳", "双碳目标+9大行业三年改造行动，高耗能行业绿色转型提速", "节能环保", 70),
    (&["新能源重卡", "重卡", "商用车电动", "电动重卡", "换电重卡", "氢能重卡", "商用车新能源", "货车电动化", "重卡换电", "电动卡车"], "新能源-重卡", "11部门推动2030年新能源重卡渗透率达40%，商用车电动化加速", "新能源重卡", 75),
];

/// 加载规则：优先 toml，不可用则回退 const。按 priority 降序返回。
fn chain_rules() -> Vec<(Vec<String>, String, String, String, u32)> {
    if let Some(config_rules) = crate::config::get_chain_rules() {
        let mut rules: Vec<_> = config_rules.into_iter().map(|r| {
            (r.keywords, r.chain, r.logic, r.board_keyword, r.priority)
        }).collect();
        rules.sort_by(|a, b| b.4.cmp(&a.4));
        return rules;
    }
    let mut rules: Vec<_> = DEFAULT_CHAIN_RULES.iter().map(|(kw, c, l, bk, p)| {
        (kw.iter().map(|s| s.to_string()).collect(), c.to_string(), l.to_string(), bk.to_string(), *p)
    }).collect();
    rules.sort_by(|a, b| b.4.cmp(&a.4));
    rules
}

/// 从新闻标题中匹配产业链（按 priority 降序遍历，高优先级规则先匹配）
pub fn map_news_to_chains(title: &str) -> Vec<ChainHit> {
    let mut hits: Vec<ChainHit> = Vec::new();
    let rules = chain_rules();

    for (keywords, chain, logic, board_keyword, _priority) in &rules {
        let matched: Vec<&str> = keywords.iter()
            .filter(|kw| title.contains(kw.as_str()))
            .map(|s| s.as_str())
            .collect();
        if matched.is_empty() { continue; }
        if hits.iter().any(|h| h.chain == *chain) { continue; }

        hits.push(ChainHit {
            chain: chain.clone(),
            keywords: matched.iter().map(|s| s.to_string()).collect(),
            logic: logic.clone(),
            stocks: Vec::new(),
            source: ChainSource::Rule,
            board_keyword: board_keyword.clone(),
            fund_flow_pct: None,
        });
    }
    hits
}

/// 新闻 → 产业链（规则优先，未命中则 AI 兜底）。
///
/// 决策（v8）：仅在关键词规则未命中时才调 AI，节省 token。
/// 数据红线 2.1/2.2：AI 不可用 → 返回空，**不编造产业链**。
pub async fn map_news_to_chains_ai(titles: &[String]) -> Vec<ChainHit> {
    let combined = titles.join(" ");
    let rule_hits = map_news_to_chains(&combined);
    if !rule_hits.is_empty() {
        return rule_hits; // 规则命中，不调 AI
    }

    // 规则未命中 → AI 兜底。
    // GeminiAnalyzer 含 RefCell（非 Sync），跨 await 会破坏外层 Future 的 Send，
    // 故隔离在独立 blocking 线程的 current-thread 运行时内执行。
    let titles_owned = titles.to_vec();
    tokio::task::spawn_blocking(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(_) => return Vec::new(),
        };
        rt.block_on(async move {
            let analyzer = crate::analyzer::GeminiAnalyzer::from_env();
            if !analyzer.is_available() {
                log::warn!("[ChainMapper] 规则未命中且 AI 不可用 → [AI降级]，不编造产业链");
                return Vec::new();
            }
            let prompt = format!(
                "你是A股产业链分析师。下面是最新快讯，请抽取其中**确有催化的产业链/概念**（没有则输出\"无\"）。\n\n<快讯>\n{}\n</快讯>\n\n要求：\n1. 最多输出3条，每条一行\n2. 格式：产业链名|催化逻辑(20字内)|板块名关键词\n3. 板块名关键词须是东方财富概念板块常见名(如 PCB、半导体、光伏、机器人)\n4. 只输出真实有逻辑的，宁缺毋滥",
                titles_owned.join("\n")
            );
            match analyzer
                .call_api_mode(&prompt, "你是A股产业链分析师,只输出格式化结果", crate::analyzer::AgentMode::Quick)
                .await
            {
                Ok(t) => parse_ai_chains(&t),
                Err(e) => {
                    log::warn!("[ChainMapper] AI 调用失败: {} → [AI降级]", e);
                    Vec::new()
                }
            }
        })
    })
    .await
    .unwrap_or_default()
}

/// 解析 AI 产出的产业链文本。格式：产业链名|催化逻辑|板块名关键词
fn parse_ai_chains(text: &str) -> Vec<ChainHit> {
    let mut hits: Vec<ChainHit> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line == "无" || line.starts_with('#') { continue; }
        let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
        if parts.len() < 3 { continue; }
        let chain = parts[0].to_string();
        let logic = parts[1].to_string();
        let board_keyword = parts[2].to_string();
        if chain.is_empty() || board_keyword.is_empty() { continue; }
        if hits.iter().any(|h| h.chain == chain) { continue; }
        hits.push(ChainHit {
            chain,
            keywords: vec![board_keyword.clone()],
            logic,
            stocks: Vec::new(),
            source: ChainSource::Ai,
            board_keyword,
            fund_flow_pct: None,
        });
        if hits.len() >= 3 { break; }
    }
    hits
}

/// 为 ChainHit 动态解析标的。
/// 先拉取全部板块排名，按板块名关键词匹配板块代码，再拉成份股。
pub fn resolve_stocks(hits: &mut [ChainHit]) {
    // 一次性拉取板块列表，构建 name→(code, 今日主力净占比) 映射
    let board_map = match sector_monitor::fetch_board_ranking("f3", 80) {
        Ok(boards) => {
            boards.into_iter()
                .map(|b| (b.name, (b.code, b.main_net_pct_today)))
                .collect::<std::collections::HashMap<String, (String, f64)>>()
        }
        Err(_) => return,
    };

    for hit in hits.iter_mut() {
        // 优先用 hit 自带的 board_keyword（Rule 来源已直接存储，AI 来源亦然）
        let mut board_keyword = hit.board_keyword.clone();
        // 安全兜底：若 board_keyword 为空，从规则表回溯查找
        if board_keyword.is_empty() {
            let rules = chain_rules();
            board_keyword = rules.iter()
                .find(|(_, chain, _, _, _)| chain == &hit.chain)
                .map(|(_, _, _, kw, _)| kw.clone())
                .unwrap_or_default();
        }

        // 空关键词会匹配任意板块，跳过以免错拉
        if board_keyword.is_empty() { continue; }

        // 动态匹配板块代码：板块名包含关键词
        let matched_board = board_map.iter()
            .find(|(name, _)| name.contains(board_keyword.as_str()))
            .map(|(_, v)| v.clone());

        let (code, flow_pct) = match matched_board {
            Some((c, flow)) => (c, flow),
            None => continue,
        };

        // 资金流向数值校验（数据红线 2.3）：净占比应在合理区间，异常值视为不可用
        hit.fund_flow_pct = if flow_pct.is_finite() && flow_pct.abs() <= 100.0 {
            Some(flow_pct)
        } else {
            None
        };

        match sector_monitor::fetch_board_components(&code, 30) {
            Ok(stocks) => {
                hit.stocks = stocks.into_iter()
                    .filter(|s| !s.code.starts_with('8') && !s.code.starts_with('4'))
                    .filter(|s| !s.code.starts_with("688"))
                    .take(15)
                    .map(|s| StockInfo { code: s.code, name: s.name, change_pct: s.change_pct, vol_ratio: s.vol_ratio })
                    .collect();
            }
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pcb_news() {
        let hits = map_news_to_chains("电子布年内第五轮提价，木林森PCB产品全线涨价20%");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chain, "AI硬件-PCB");
    }

    #[test]
    fn test_multi_chain_news() {
        let hits = map_news_to_chains("MLCC突破带动PCB和半导体产业链全线走强");
        assert!(hits.iter().any(|h| h.chain == "AI硬件-MLCC"));
        assert!(hits.iter().any(|h| h.chain == "AI硬件-PCB"));
    }

    #[test]
    fn test_no_match() {
        let hits = map_news_to_chains("今日天气晴朗适合出游");
        assert!(hits.is_empty());
    }

    #[test]
    fn test_city_renewal() {
        let hits = map_news_to_chains("国务院通过城市更新十五五规划，地下管网改造加速");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chain, "城市更新");
    }

    #[test]
    fn test_rule_hit_marks_source_rule() {
        let hits = map_news_to_chains("电子布提价带动PCB涨价");
        assert_eq!(hits[0].source, ChainSource::Rule);
    }

    #[test]
    fn test_parse_ai_chains_ok() {
        let text = "固态电池|技术迭代催化|固态电池\n机器人|人形量产提速|机器人\n无效行";
        let hits = parse_ai_chains(text);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].chain, "固态电池");
        assert_eq!(hits[0].source, ChainSource::Ai);
        assert_eq!(hits[0].board_keyword, "固态电池");
        assert!(hits.iter().all(|h| h.source == ChainSource::Ai));
    }

    #[test]
    fn test_parse_ai_chains_empty_and_no() {
        assert!(parse_ai_chains("无").is_empty());
        assert!(parse_ai_chains("").is_empty());
        // 板块关键词为空 → 丢弃，不伪造
        assert!(parse_ai_chains("某链|某逻辑|").is_empty());
    }

    #[tokio::test]
    async fn test_ai_fallback_skips_ai_when_rule_hits() {
        // 规则命中时直接返回规则结果，不触发 AI（即使 AI 不可用也能产出）
        let hits = map_news_to_chains_ai(&["PCB全线涨价20%".to_string()]).await;
        assert!(!hits.is_empty());
        assert_eq!(hits[0].source, ChainSource::Rule);
    }

    #[test]
    fn test_hbm_news() {
        let hits = map_news_to_chains("SK海力士HBM3E量产供货英伟达，高带宽内存需求爆发");
        assert!(hits.iter().any(|h| h.chain == "HBM-高带宽内存"));
    }

    #[test]
    fn test_commercial_aerospace() {
        let hits = map_news_to_chains("千帆星座第二批卫星成功发射，商业航天低轨组网加速");
        assert!(hits.iter().any(|h| h.chain == "商业航天"));
    }

    #[test]
    fn test_solid_state_battery_separate_from_lithium() {
        // 固态电池应命中独立规则，而非笼统的 新能源-锂电池
        let hits = map_news_to_chains("丰田宣布硫化物固态电池2027年量产装车");
        assert!(hits.iter().any(|h| h.chain == "新能源-固态电池"));
    }

    #[test]
    fn test_smart_driving_news() {
        let hits = map_news_to_chains("特斯拉FSD入华获批，端到端智驾加速落地");
        assert!(hits.iter().any(|h| h.chain == "智能驾驶"));
    }

    #[test]
    fn test_board_keyword_stored_for_rule_hits() {
        // v2: rule 来源的 hit 应直接携带 board_keyword，不再依赖 resolve 阶段二次查表
        let hits = map_news_to_chains("CPO光模块出货量翻倍，1.6T产品验证通过");
        let cpo_hit = hits.iter().find(|h| h.chain == "AI硬件-CPO").unwrap();
        assert_eq!(cpo_hit.board_keyword, "CPO");
        assert!(!cpo_hit.board_keyword.is_empty());
    }

    #[test]
    fn test_new_energy_hydrogen() {
        let hits = map_news_to_chains("绿氢项目批量获批，PEM电解槽需求爆发在即");
        assert!(hits.iter().any(|h| h.chain == "新能源-氢能"));
    }

    #[test]
    fn test_rare_earth_magnets() {
        let hits = map_news_to_chains("稀土配额收紧叠加人形机器人放量，钕铁硼磁材供需缺口扩大");
        assert!(hits.iter().any(|h| h.chain == "稀土永磁"));
    }

    #[test]
    fn test_quantum_computing() {
        let hits = map_news_to_chains("中国量子计算原型机实现1000量子比特突破");
        assert!(hits.iter().any(|h| h.chain == "量子计算"));
    }
}
