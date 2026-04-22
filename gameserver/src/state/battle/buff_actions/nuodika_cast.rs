//! Handlers for buff_act 1006 NuoDiKaCastChannel + 1010 DyingHealDisperse1 — Nautika's Divine Yoik channel + per-hit variant (Bug C 17581×2 pattern).

#[allow(dead_code)]
pub const BUFF_ACT_ID_CAST_CHANNEL: i32 = 1006;
#[allow(dead_code)]
pub const BUFF_ACT_ID_DYING_HEAL_DISPERSE: i32 = 1010;

pub fn buff_get_nuodika_channel_params(buff_id: i32) -> Option<(i32, i32, i32, i32, i32, i32)> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    for segment in buff.features.split('|') {
        let parts: Vec<&str> = segment.split('#').collect();
        let feature_id: i32 = parts.first()?.trim().parse().ok()?;
        let Some(act_type) = cfg
            .buff_act
            .iter()
            .find(|a| a.id == feature_id)
            .map(|a| a.r#type.as_str())
        else {
            continue;
        };
        if act_type != "NuoDiKaCastChannel" {
            continue;
        }
        let duration = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        let threshold = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
        let max_points = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
        let points_per_trigger = parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0);
        let output_skill_id = parts.get(5).and_then(|v| v.parse().ok()).unwrap_or(0);
        let counter_buff_id = parts.get(6).and_then(|v| v.parse().ok()).unwrap_or(0);
        if threshold > 0 && points_per_trigger > 0 && output_skill_id > 0 {
            return Some((
                duration,
                threshold,
                max_points,
                points_per_trigger,
                output_skill_id,
                counter_buff_id,
            ));
        }
    }
    None
}
