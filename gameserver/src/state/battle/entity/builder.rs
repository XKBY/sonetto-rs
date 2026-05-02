use database::{db::game::equipment::Equipment, models::game::heros::HeroData};
use sonettobuf::{EnhanceInfoBox, EquipRecord, FightEntityInfo, HeroAttribute};

use super::{attr::Attr, destiny::Destiny, passive::Passive, skill::Skill};

pub struct EntityBuilder {
    hero_data: HeroData,
    equip: Option<Equipment>,
    position: i32,
    team_type: i32,
    is_sub: bool,
}

impl EntityBuilder {
    pub fn new(hero_data: HeroData, position: i32, team_type: i32, is_sub: bool) -> Self {
        Self {
            hero_data,
            equip: None,
            position,
            team_type,
            is_sub,
        }
    }

    pub fn with_equip(mut self, equip: Equipment) -> Self {
        self.equip = Some(equip);
        self
    }

    pub fn build(self) -> FightEntityInfo {
        let r = &self.hero_data.record;

        let destiny = Destiny::get(r.destiny_stone, r.destiny_rank);
        let attr = Attr::get(&self.hero_data, self.equip.as_ref());
        let (sg1, sg2) = Skill::get(&self.hero_data, self.is_sub, destiny.as_ref());
        let passives = Passive::get(
            &self.hero_data,
            self.equip.as_ref().map(|e| e.equip_id),
            destiny.as_ref(),
            r.destiny_stone,
            r.destiny_rank,
        );
        let ex_skill = Skill::get_ex(&self.hero_data, destiny.as_ref());
        let current_hp = attr.hp.unwrap_or(0);

        let equip_record = EquipRecord {
            equip_uid: self.equip.as_ref().map(|e| e.uid),
            equip_id: self.equip.as_ref().map(|e| e.equip_id),
            equip_lv: self.equip.as_ref().map(|e| e.level),
            refine_lv: self.equip.as_ref().map(|e| e.refine_lv),
        };

        FightEntityInfo {
            uid: Some(r.uid),
            model_id: Some(r.hero_id),
            skin: Some(r.skin),
            position: Some(self.position),
            entity_type: Some(1),
            user_id: Some(r.user_id),
            ex_point: Some(0),
            level: Some(r.level),
            current_hp: Some(current_hp),
            attr: Some(attr),
            base_attr: Some(attr),
            buffs: vec![],
            skill_group1: sg1,
            skill_group2: sg2,
            passive_skill: passives,
            ex_skill: Some(ex_skill),
            shield_value: Some(0),
            no_effect_buffs: vec![],
            expoint_max_add: Some(0),
            buff_harm_statistic: Some(0),
            equip_uid: Some(r.default_equip_uid),
            trial_equip: Some(EquipRecord {
                equip_uid: Some(0),
                equip_id: Some(0),
                equip_lv: Some(0),
                refine_lv: Some(0),
            }),
            ex_skill_level: Some(r.ex_skill_level),
            power_infos: vec![],
            act104_equip_uids: vec![],
            trial_act104_equips: vec![],
            summoned_list: vec![],
            ex_skill_point_change: Some(0),
            team_type: Some(self.team_type),
            enhance_info_box: Some(EnhanceInfoBox {
                uid: Some(r.uid),
                can_upgrade_ids: vec![],
                upgraded_options: vec![],
            }),
            trial_id: Some(0),
            career: Some(Self::career(&self.hero_data)),
            status: Some(0),
            guard: Some(-1),
            sub_cd: Some(0),
            ex_point_type: Some(Self::ex_point_type(r.hero_id)),
            equips: vec![equip_record],
            destiny_stone: Some(r.destiny_stone),
            destiny_rank: Some(r.destiny_rank),
            custom_unit_id: Some(0),
        }
    }

    pub fn player(user_id: i64, team_type: i32) -> FightEntityInfo {
        let uid = if team_type == 1 { 0 } else { -99999 };

        let attr = HeroAttribute {
            hp: Some(100),
            attack: Some(0),
            defense: Some(0),
            mdefense: Some(0),
            technic: Some(0),
            multi_hp_idx: Some(0),
            multi_hp_num: Some(0),
        };

        FightEntityInfo {
            uid: Some(uid),
            model_id: Some(0),
            skin: Some(0),
            position: Some(0),
            entity_type: Some(3),
            user_id: Some(user_id),
            ex_point: Some(0),
            level: Some(0),
            current_hp: Some(100),
            attr: Some(attr),
            buffs: vec![],
            skill_group1: vec![],
            skill_group2: vec![],
            passive_skill: vec![],
            ex_skill: Some(0),
            shield_value: Some(0),
            no_effect_buffs: vec![],
            expoint_max_add: Some(0),
            buff_harm_statistic: Some(0),
            equip_uid: Some(0),
            trial_equip: Some(EquipRecord {
                equip_uid: Some(0),
                equip_id: Some(0),
                equip_lv: Some(0),
                refine_lv: Some(0),
            }),
            ex_skill_level: Some(0),
            power_infos: vec![],
            act104_equip_uids: vec![],
            trial_act104_equips: vec![],
            summoned_list: vec![],
            base_attr: Some(attr),
            ex_skill_point_change: Some(0),
            team_type: Some(team_type),
            enhance_info_box: Some(sonettobuf::EnhanceInfoBox {
                uid: Some(uid),
                can_upgrade_ids: vec![],
                upgraded_options: vec![],
            }),
            trial_id: Some(0),
            career: Some(0),
            status: Some(0),
            guard: Some(-1),
            sub_cd: Some(0),
            ex_point_type: Some(0),
            equips: vec![],
            destiny_stone: Some(0),
            destiny_rank: Some(0),
            custom_unit_id: Some(0),
        }
    }

    fn ex_point_type(hero_id: i32) -> i32 {
        match hero_id {
            3120 => 1,
            3123 => 2,
            3124 | 3122 => 3,
            _ => 0,
        }
    }

    fn career(hero_data: &HeroData) -> i32 {
        use config::configs;
        configs::get()
            .character
            .iter()
            .find(|c| c.id == hero_data.record.hero_id)
            .map(|c| c.career)
            .unwrap_or(0)
    }
}
