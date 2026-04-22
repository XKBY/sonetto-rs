//! Handler for buff_act 865 AddPassiveSkills.
//!
//! This feature adds virtual passive skill ids while the source buff remains
//! active. Runtime consumers use it to expand effective passive-skill sets and
//! to infer follow-up/precast behavior from those injected passives.

#[allow(dead_code)]
pub const BUFF_ACT_ID: i32 = 865;

pub fn for_each_add_passive_skill_id(buff_id: i32, mut f: impl FnMut(i32)) {
    crate::state::battle::utils::for_each_buff_feature_chain(buff_id, |act_type, parts| {
        if act_type != "AddPassiveSkills" {
            return;
        }
        for raw in parts.iter().skip(1) {
            for piece in raw.split(',') {
                let Ok(skill_id) = piece.trim().parse::<i32>() else {
                    continue;
                };
                if skill_id > 0 {
                    f(skill_id);
                }
            }
        }
    });
}
