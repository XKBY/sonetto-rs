use sonettobuf::CardInfo;

pub fn make_card(hero_id: i32, skill_id: i32, uid: i64, is_trial: bool) -> CardInfo {
    CardInfo {
        uid: Some(uid),
        hero_id: Some(hero_id),
        skill_id: Some(skill_id),
        card_type: Some(0),
        status: Some(0),
        temp_card: Some(is_trial),
        enchants: vec![],
        target_uid: Some(0),
        energy: Some(0),
        extra_infos: vec![],
        area_red_or_blue: Some(0),
        heat_id: Some(0),
        card_effect: None,
        extra_info: None,
        music_note: None,
    }
}
