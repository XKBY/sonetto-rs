use super::action::{BuffActCtx, BuffActionHandler, BuffStage};
use super::result::ActionResult;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProbabilityAddBuffSpec {
    pub permille: i32,
    pub buff_id: i32,
    pub stack_count: i32,
}

pub(super) struct ProbabilityAddBuffHandler;

impl BuffActionHandler for ProbabilityAddBuffHandler {
    type Params = ();

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "ProbabilityAddBuff" && stage == BuffStage::AfterBuffAdd
    }

    fn parse(&self, _parts: &[&str], _ctx: &BuffActCtx<'_, '_>) -> Self::Params {}

    fn steps(&self, _params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        ActionResult::empty()
    }
}

pub(crate) fn probability_add_buff_specs(buff_id: i32) -> Vec<ProbabilityAddBuffSpec> {
    let cfg = config::configs::get();
    let Some(buff_cfg) = cfg.skill_buff.iter().find(|row| row.id == buff_id) else {
        return Vec::new();
    };

    buff_cfg
        .features
        .split('|')
        .filter_map(|entry| {
            let parts: Vec<&str> = entry.split('#').collect();
            let act_id = parts
                .first()
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0);
            let act_type = cfg
                .buff_act
                .iter()
                .find(|row| row.id == act_id)
                .map(|row| row.r#type.as_str())
                .unwrap_or("");
            if act_type != "ProbabilityAddBuff" {
                return None;
            }

            let permille = parts
                .get(1)
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0);
            let trigger_buff_id = parts
                .get(2)
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0);
            let stack_count = parts
                .get(3)
                .and_then(|v| v.split(',').next())
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(1)
                .max(1);
            if trigger_buff_id <= 0 {
                return None;
            }

            Some(ProbabilityAddBuffSpec {
                permille,
                buff_id: trigger_buff_id,
                stack_count,
            })
        })
        .collect()
}
