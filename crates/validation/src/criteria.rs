//! 准则注册表（validation.md §5）。
//!
//! 每个编号是稳定准则族，实施为 `family/check` 二级寻址。注册表覆盖
//! 五层目录的全部叶项编号；判定逻辑在各族驱动器中实现，注册表负责
//! 编号、门类与描述的单一事实源。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::verdict::GateClass;

/// 准则族（validation.md §5 目录前缀）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Family {
    /// 第 1 层流程完整性。
    Flow,
    /// 第 2 层事实与账本。
    Cause,
    /// 球员运动。
    Move,
    /// 战术。
    Tact,
    /// 第 3 层数据、指标和统计判定。
    Stat,
    /// 第 4 层机制。
    Mech,
    /// 第 5 层联合与连续过程。
    Joint,
}

impl Family {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Flow => "FLOW",
            Self::Cause => "CAUSE",
            Self::Move => "MOVE",
            Self::Tact => "TACT",
            Self::Stat => "STAT",
            Self::Mech => "MECH",
            Self::Joint => "JOINT",
        }
    }
}

/// 一条注册的叶项：稳定编号、门类与义务描述。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Criterion {
    /// 形如 `FLOW-A01` 的稳定编号。
    pub id: String,
    pub family: Family,
    pub gate_class: GateClass,
    /// 判据一句话描述（validation.md §5–§8 目录原文语义）。
    pub description: String,
}

impl Criterion {
    fn new(id: &str, family: Family, gate_class: GateClass, description: &str) -> Self {
        Self {
            id: id.to_string(),
            family,
            gate_class,
            description: description.to_string(),
        }
    }
}

/// 全量准则注册表：编号到叶项的唯一映射。
#[derive(Debug, Clone, Default)]
pub struct CriterionRegistry {
    entries: BTreeMap<String, Criterion>,
}

impl CriterionRegistry {
    /// 构建 validation.md §5–§8 目录的全量注册表。
    pub fn full() -> Self {
        let mut registry = Self::default();
        registry.register_flow();
        registry.register_cause();
        registry.register_move();
        registry.register_tact();
        registry.register_stat();
        registry.register_mech();
        registry.register_joint();
        registry
    }

    pub fn get(&self, id: &str) -> Option<&Criterion> {
        self.entries.get(id)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    pub fn by_family(&self, family: Family) -> Vec<&Criterion> {
        self.entries
            .values()
            .filter(|entry| entry.family == family)
            .collect()
    }

    fn register(&mut self, criterion: Criterion) {
        let duplicated = self
            .entries
            .insert(criterion.id.clone(), criterion)
            .is_some();
        debug_assert!(!duplicated, "criterion id registered twice");
    }

    /// §5.1 第 1 层流程目录：全部 `FLOW` 叶项为 `Integrity`。
    fn register_flow(&mut self) {
        let g = GateClass::Integrity;
        let items = [
            ("FLOW-A01", "动作开始后恰有一个终态；窗口按业务身份清算"),
            ("FLOW-A02", "动作阶段合法；中断只关闭尚可被中断的动作部分"),
            (
                "FLOW-A03",
                "十三类动作窗口逐类登记正常、阻断、判罚和回合边界适用测试",
            ),
            (
                "FLOW-A04",
                "窗口完成、移动到位、接球与投篮结果分别结算，禁止相互替代",
            ),
            (
                "FLOW-S01",
                "每次投篮只有一个最终裁决，全部物理与规则分支有出口",
            ),
            (
                "FLOW-S02",
                "离手前后触球分别验证；合法封盖、干扰球与篮筐干涉按规则区分",
            ),
            (
                "FLOW-S03",
                "投失之后核对控制、出界、争球或判罚；篮板归属由控制建立确定",
            ),
            (
                "FLOW-S04",
                "进攻篮板延续回合；进攻时钟读取触筐与控制条件；防守篮板开启对方进攻",
            ),
            ("FLOW-S05", "投篮犯规各组合分别核对判罚、得分与投篮记账"),
            (
                "FLOW-P01",
                "传球在接住、截获、出界、规则取消或转为松球时结束原飞行",
            ),
            (
                "FLOW-P02",
                "冻结目标、实际接球位置及修正事实按 pass_id 核对",
            ),
            (
                "FLOW-P03",
                "哨响中断的传球有 InterruptedBy，不自动记为技术失误",
            ),
            ("FLOW-D01", "突破各终态明确；分球无需先出现到达篮下事件"),
            ("FLOW-L01", "所有松球实例以控制、出界、争球或判罚关闭"),
            ("FLOW-L02", "暂时无持球者不自动改变统计回合"),
            ("FLOW-R01", "判罚逐类覆盖；规则依据和队内责任明确"),
            (
                "FLOW-R02",
                "罚球程序保留来源、执行人、尝试序号、重罚与有效次数",
            ),
            ("FLOW-R03", "时限与违例规则按联赛能力目录逐项检查"),
            ("FLOW-R04", "换人须满足具体换人许可条件"),
            (
                "FLOW-R05",
                "时钟截止后等待待决投篮和罚则，结算比分后才决定终局",
            ),
        ];
        for (id, description) in items {
            self.register(Criterion::new(id, Family::Flow, g, description));
        }
    }

    /// §5.2 第 2 层事实与账本目录：全部 `CAUSE` 叶项为 `Integrity`。
    fn register_cause(&mut self) {
        let g = GateClass::Integrity;
        let items = [
            (
                "CAUSE-I01",
                "唯一事件身份、顺序、无环与关系基数；业务实例只结算一次",
            ),
            ("CAUSE-I02", "投篮来源分别核查；主要来源与附加来源分开保存"),
            ("CAUSE-I03", "按 §3.2 验证跨回合及跨球队关系"),
            ("CAUSE-I04", "传球、投篮、动作、罚球程序和控制事实配对完整"),
            (
                "CAUSE-B01",
                "独立重建双方及球员箱体十三项，与引擎统计逐项比较",
            ),
            (
                "CAUSE-B02",
                "独立球权和规范统计回合与 PossessionSummary 比较",
            ),
            ("CAUSE-B03", "三种时长、时钟运行与重置分别重建"),
            (
                "CAUSE-B04",
                "个人与球队责任、罚球资格、犯规与奖励状态独立重算",
            ),
        ];
        for (id, description) in items {
            self.register(Criterion::new(id, Family::Cause, g, description));
        }
    }

    /// §5.3 球员运动目录：各族注册 `/fact`（Integrity）与适用的
    /// `/profile`（Distribution）或 `/response`（Mechanism）变体。
    fn register_move(&mut self) {
        let fact = GateClass::Integrity;
        let profile = GateClass::Distribution;
        let response = GateClass::Mechanism;
        let families = [
            (
                "MOVE-H01",
                "持球人和篮球连续性、速度、加速度、边界、持球合法性",
                true,
                true,
                false,
            ),
            (
                "MOVE-H02",
                "推进和突破的任务、真实轨迹、到达或阻断证据",
                true,
                true,
                false,
            ),
            (
                "MOVE-H03",
                "利用掩护的路线、屏障几何与防守绕行；拒绝掩护也是合法选择",
                true,
                true,
                false,
            ),
            (
                "MOVE-H04",
                "已有机会下停留、出手或传球的采用率，按时钟和比分分组",
                true,
                true,
                true,
            ),
            (
                "MOVE-O01",
                "切入、背切、转移、定点、掩护、顺下、外弹、卡位与补篮的任务和终态",
                true,
                true,
                false,
            ),
            (
                "MOVE-O02",
                "切入接球的窗口、实际控制及身份一致性",
                true,
                true,
                false,
            ),
            (
                "MOVE-O03",
                "掩护建立、滑出、提前离开、顺下、外弹与阻断分别测量",
                true,
                true,
                false,
            ),
            ("MOVE-O04", "冲抢与退防任务分别评价", true, true, true),
            (
                "MOVE-F01",
                "真实防守距离、可达时间与干扰记录",
                true,
                true,
                false,
            ),
            (
                "MOVE-F02",
                "挤过、绕过、沉退、延误、换防、协防、回位任务与真实运动一致",
                true,
                true,
                false,
            ),
            (
                "MOVE-F03",
                "责任分配、移交与执行成功分别记录；伪报完成才是结构缺陷",
                true,
                true,
                true,
            ),
            (
                "MOVE-F04",
                "抢断、封盖、卡位及篮板控制的几何和接触证据",
                true,
                true,
                false,
            ),
        ];
        for (base, description, has_fact, has_profile, has_response) in families {
            if has_fact {
                self.register(Criterion::new(
                    &format!("{base}/fact"),
                    Family::Move,
                    fact,
                    description,
                ));
            }
            if has_profile {
                self.register(Criterion::new(
                    &format!("{base}/profile"),
                    Family::Move,
                    profile,
                    description,
                ));
            }
            if has_response {
                self.register(Criterion::new(
                    &format!("{base}/response"),
                    Family::Move,
                    response,
                    description,
                ));
            }
        }
    }

    /// §5.4 战术目录：`/fact` 为 Integrity，`/response` 为 Mechanism，
    /// `/profile` 为 Distribution。
    fn register_tact(&mut self) {
        let fact = GateClass::Integrity;
        let profile = GateClass::Distribution;
        let response = GateClass::Mechanism;
        let families = [
            (
                "TACT-O01",
                "槽位指派完整、人员资格合法、换人后引用有效",
                true,
                false,
                false,
            ),
            (
                "TACT-O02",
                "半场组织中的队友间距、禁区占用、传球窗口与弱侧空间",
                false,
                true,
                false,
            ),
            (
                "TACT-O03",
                "外线展开、高位掩护、切入、低位组织等体系的机会构成",
                false,
                true,
                false,
            ),
            (
                "TACT-O04",
                "固定阵容只改变体系时各指标的变化分别报告",
                false,
                false,
                true,
            ),
            (
                "TACT-P01",
                "触发谓词、竞争选择、激活窗口、冷却和身份一致",
                true,
                false,
                false,
            ),
            (
                "TACT-P02",
                "有效窗口内施加声明的候选增量与约束；结束后停止新增修正",
                true,
                false,
                false,
            ),
            (
                "TACT-P03",
                "规则匹配、目标修订、实际跑位、机会和选择逐环节可追溯",
                true,
                false,
                true,
            ),
            (
                "TACT-P04",
                "未能形成掩护、被防守破坏、判罚中断、形成机会后投失分别计数",
                false,
                true,
                false,
            ),
            (
                "TACT-F01",
                "同一掩护面对不同防守方案核对责任与条件空间分布",
                true,
                true,
                false,
            ),
            (
                "TACT-F02",
                "对位防守和区域防守分别核对对象或区域责任",
                true,
                true,
                false,
            ),
            (
                "TACT-F03",
                "认证进攻体系与防守体系按兼容矩阵交叉比较",
                false,
                true,
                false,
            ),
            (
                "TACT-F04",
                "教练调整的来源、有效时间及后续实际响应",
                true,
                false,
                true,
            ),
        ];
        for (base, description, has_fact, has_profile, has_response) in families {
            if has_fact {
                self.register(Criterion::new(
                    &format!("{base}/fact"),
                    Family::Tact,
                    fact,
                    description,
                ));
            }
            if has_profile {
                self.register(Criterion::new(
                    &format!("{base}/profile"),
                    Family::Tact,
                    profile,
                    description,
                ));
            }
            if has_response {
                self.register(Criterion::new(
                    &format!("{base}/response"),
                    Family::Tact,
                    response,
                    description,
                ));
            }
        }
    }

    /// §6.2 第 3 层指标族：数据依赖未满足时统一 `InsufficientEvidence`，
    /// 门类全部为 `Distribution`。
    fn register_stat(&mut self) {
        let g = GateClass::Distribution;
        let items = [
            ("STAT-G01", "球队得分、总分、分差与进攻效率"),
            ("STAT-G02", "球队回合数与 Pace"),
            ("STAT-S01", "命中率、三分率、四区出手份额与各区命中率"),
            ("STAT-S02", "出手来源、转换时间与防守距离条件下的选择与命中"),
            ("STAT-R01", "罚球率、犯规率与奖励阶段罚球"),
            ("STAT-T01", "失误率与规范失误原因份额"),
            ("STAT-B01", "进攻篮板率与前场篮板后出手率"),
            ("STAT-P01", "每回合传球数、传球结果占比与机会采用率"),
            ("STAT-M01", "轨迹长度、速度分位数、到位时间等帧指标"),
            ("STAT-I01", "球员上场分钟、得分出手份额与使用率"),
        ];
        for (id, description) in items {
            self.register(Criterion::new(id, Family::Stat, g, description));
        }
    }

    /// §7.3 输入覆盖目录：每个输入的机制叶项，`Mechanism` 门类。
    fn register_mech(&mut self) {
        let g = GateClass::Mechanism;
        let items = [
            ("MECH-S01", "区域精度与选择对 shooting 输入的响应"),
            (
                "MECH-S02",
                "终结、接触高度、合法触球与结果对 finishing/vertical/block 的响应",
            ),
            (
                "MECH-P01",
                "传球执行、接球和控制丧失对 passing/ball_handling 的响应",
            ),
            ("MECH-M01", "运动、感知、体能消耗与恢复对运动类输入的响应"),
            ("MECH-B01", "对抗与篮板控制对 strength/rebound 输入的响应"),
            ("MECH-F01", "防守机会、选择及代价对防守类输入的响应"),
            ("MECH-R01", "罚球与决策质量对 free_throw/decision_iq 的响应"),
            ("MECH-T01", "倾向采用率对 tendency 输入的响应"),
            ("MECH-T02", "体系、Play 与战术谓词的实际消费及结果"),
            ("MECH-C01", "球队风格输入与教练、轮换输入的响应"),
        ];
        for (id, description) in items {
            self.register(Criterion::new(id, Family::Mech, g, description));
        }
    }

    /// §8 第 5 层联合与连续过程族，`Distribution` 门类。
    fn register_joint(&mut self) {
        let g = GateClass::Distribution;
        let items = [
            (
                "JOINT-C01",
                "比分差、剩余时间与进攻时钟共同条件下的节奏与战术",
            ),
            (
                "JOINT-C02",
                "各转换类型的推进和回防人数、第一出手时间及结果",
            ),
            ("JOINT-C03", "突破、外线空间、协防与传球出口的联合分布"),
            ("JOINT-C04", "出手位置、球运动、卡位与前场篮板后二次进攻"),
            ("JOINT-C05", "犯规账期、奖励状态、对抗选择及罚球"),
            ("JOINT-T01", "进攻体系与防守方案的全体兼容组合"),
            ("JOINT-T02", "Play 激活、机会生成、采用、结果及中断类型"),
            ("JOINT-I01", "球员能力、使用机会、上场时间与队友对手关系"),
            ("JOINT-Q01", "连续回合的转移矩阵与 lag 条件统计"),
        ];
        for (id, description) in items {
            self.register(Criterion::new(id, Family::Joint, g, description));
        }
    }
}
