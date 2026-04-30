//! buff_act 1006 NuoDiKaCastChannel + 1010 DyingHealDisperse1 —
//! Nautika's Divine Yoik channel + per-hit variant.

#[allow(dead_code)]
pub const BUFF_ACT_ID_CAST_CHANNEL: i32 = 1006;
#[allow(dead_code)]
pub const BUFF_ACT_ID_DYING_HEAL_DISPERSE: i32 = 1010;

/// Returns `(duration, threshold, max_points, points_per_trigger,
/// output_skill_id, counter_buff_id)` or `None` if the buff isn't
/// carrying this feature or any required field is 0.
pub fn buff_get_nuodika_channel_params(buff_id: i32) -> Option<(i32, i32, i32, i32, i32, i32)> {
    let parts = super::find_feature_parts(buff_id, "NuoDiKaCastChannel")?;
    let duration = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let threshold = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
    let max_points = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
    let points_per_trigger = parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0);
    let output_skill_id = parts.get(5).and_then(|v| v.parse().ok()).unwrap_or(0);
    let counter_buff_id = parts.get(6).and_then(|v| v.parse().ok()).unwrap_or(0);
    if threshold > 0 && points_per_trigger > 0 && output_skill_id > 0 {
        Some((
            duration,
            threshold,
            max_points,
            points_per_trigger,
            output_skill_id,
            counter_buff_id,
        ))
    } else {
        None
    }
}
