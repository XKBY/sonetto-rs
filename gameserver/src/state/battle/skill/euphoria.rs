use sonettobuf::Fight;

use crate::state::battle::entity::destiny::Destiny;

use super::{cache::resolve_skill_effect_id, targets::get_entity};

pub fn resolve_with_euphoria(fight: &Fight, owner_uid: i64, base_skill_id: i32) -> i32 {
    if owner_uid == 0 || base_skill_id <= 0 {
        return base_skill_id;
    }

    let Some(entity) = get_entity(fight, owner_uid) else {
        return base_skill_id;
    };

    let rank = entity.destiny_rank.unwrap_or(0).max(0);
    if rank <= 0 {
        return base_skill_id;
    }

    let facets_id = entity
        .destiny_stone
        .filter(|value| *value > 0)
        .or_else(|| entity.model_id.and_then(Destiny::facets_id_for_hero))
        .unwrap_or(0);
    if facets_id <= 0 {
        return base_skill_id;
    }

    Destiny::resolve_skill_id(facets_id, rank, base_skill_id)
}

pub fn resolve_skill_effect_id_for_entity(fight: &Fight, owner_uid: i64, skill_id: i32) -> i32 {
    resolve_skill_effect_id(resolve_with_euphoria(fight, owner_uid, skill_id))
}
