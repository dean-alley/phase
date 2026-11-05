//! Tests relocated from `src/parser/oracle_casting.rs`'s test module.

use engine::types::ability::Effect;

/// CR 601.3: Even the Odds (verbatim). Its "Cast this spell only if …" condition is
/// not representable as a restriction, so the line must be an honest coverage gap —
/// never a conditional self-recast (`CastFromZone`) that drops the restriction —
/// while the spell's own instruction still parses on its own.
#[test]
fn unrepresentable_cast_only_condition_is_a_gap_not_a_self_recast() {
    let text = "Cast this spell only if you control fewer creatures than each opponent.\nCreate three 1/1 white Soldier creature tokens.";
    let parsed = engine::parser::oracle::parse_oracle_text(
        text,
        "Even the Odds",
        &[],
        &["Instant".to_string()],
        &[],
    );
    let mut effects = Vec::new();
    for ability in &parsed.abilities {
        let _ = engine::types::ability_visit::visit_ability_def(ability, &mut |effect| {
            effects.push(effect.clone());
            std::ops::ControlFlow::Continue(())
        });
    }
    // Reach-guard: the resolution instruction is still parsed.
    assert!(
        effects.iter().any(|e| matches!(e, Effect::Token { .. })),
        "{effects:?}"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::CastFromZone { .. })),
        "{effects:?}"
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::Unimplemented { name, .. } if name == "casting_restriction"
        )),
        "{effects:?}"
    );
    assert!(parsed.casting_restrictions.is_empty());
    let face = engine::types::card::CardFace {
        name: "Even the Odds".to_string(),
        oracle_text: Some(text.to_string()),
        abilities: parsed.abilities,
        ..Default::default()
    };
    assert!(!engine::game::coverage::card_face_gaps(&face).is_empty());
}

/// Every effect in every ability tree of `abilities`, collected through the shared
/// ability visitor.
fn all_effects(abilities: &[engine::types::ability::AbilityDefinition]) -> Vec<Effect> {
    let mut effects = Vec::new();
    for ability in abilities {
        let _ = engine::types::ability_visit::visit_ability_def(ability, &mut |effect| {
            effects.push(effect.clone());
            std::ops::ControlFlow::Continue(())
        });
    }
    effects
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

/// CR 118.9 + CR 207.2c: Admiral's Order (verbatim). The ability word "Raid" has no
/// rules meaning, so the labeled line is a raid-gated {U} alternative cost and the
/// spell ability is only "Counter target spell" — never a resolution `PayCost`.
#[test]
fn admirals_order_raid_line_is_a_casting_option() {
    use engine::types::ability::{
        AbilityCost, Comparator, CountScope, ParsedCondition, QuantityExpr, QuantityRef,
        SpellCastingOptionKind,
    };
    use engine::types::mana::{ManaCost, ManaCostShard};

    let text = "Raid — If you attacked this turn, you may pay {U} rather than pay this spell's mana cost.\nCounter target spell.";
    let parsed = engine::parser::oracle::parse_oracle_text(
        text,
        "Admiral's Order",
        &[],
        &strings(&["Instant"]),
        &[],
    );
    assert_eq!(
        parsed.casting_options.len(),
        1,
        "{:?}",
        parsed.casting_options
    );
    let option = &parsed.casting_options[0];
    assert_eq!(option.kind, SpellCastingOptionKind::AlternativeCost);
    assert_eq!(
        option.cost,
        Some(AbilityCost::Mana {
            cost: ManaCost::Cost {
                shards: vec![ManaCostShard::Blue],
                generic: 0,
            },
        })
    );
    assert_eq!(
        option.condition,
        Some(ParsedCondition::QuantityComparison {
            lhs: QuantityExpr::Ref {
                qty: QuantityRef::AttackedThisTurn {
                    scope: CountScope::Controller,
                    filter: None,
                },
            },
            comparator: Comparator::GE,
            rhs: QuantityExpr::Fixed { value: 1 },
        })
    );

    let effects = all_effects(&parsed.abilities);
    // Reach-guard for the negatives: the line set parsed with no gap at all.
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Unimplemented { .. })),
        "{effects:?}"
    );
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::PayCost { .. })),
        "{effects:?}"
    );
    assert_eq!(parsed.abilities.len(), 1, "{:?}", parsed.abilities);
    assert!(
        matches!(*parsed.abilities[0].effect, Effect::Counter { .. }),
        "{:?}",
        parsed.abilities[0]
    );
    assert_eq!(effects.len(), 1, "{effects:?}");
}

/// CR 601.3d + CR 207.2c: Dragon Grip's ferocious and Swift Reckoning's spell mastery
/// lines (verbatim) are conditional flash permissions read past their ability words —
/// never a spell-resolution `CastFromZone`.
#[test]
fn labeled_flash_permission_is_a_casting_option() {
    use engine::types::ability::{
        Comparator, ParsedCondition, QuantityExpr, QuantityRef, SpellCastingOptionKind,
    };

    let grip_text = "Ferocious — If you control a creature with power 4 or greater, you may cast this spell as though it had flash. (You may cast it any time you could cast an instant.)\nEnchant creature\nEnchanted creature gets +2/+0 and has first strike.";
    let grip = engine::parser::oracle::parse_oracle_text(
        grip_text,
        "Dragon Grip",
        &strings(&["Enchant"]),
        &strings(&["Enchantment"]),
        &strings(&["Aura"]),
    );
    assert_eq!(grip.casting_options.len(), 1, "{:?}", grip.casting_options);
    assert_eq!(
        grip.casting_options[0].kind,
        SpellCastingOptionKind::AsThoughHadFlash
    );
    assert_eq!(grip.casting_options[0].cost, None);
    assert!(
        matches!(
            grip.casting_options[0].condition,
            Some(ParsedCondition::QuantityComparison {
                lhs: QuantityExpr::Ref {
                    qty: QuantityRef::ObjectCount { .. },
                },
                comparator: Comparator::GE,
                rhs: QuantityExpr::Fixed { value: 1 },
            })
        ),
        "{:?}",
        grip.casting_options[0]
    );
    assert!(grip.abilities.is_empty(), "{:?}", grip.abilities);
    // Positive: the enchanted-creature static still parses.
    assert!(!grip.statics.is_empty());

    let swift_text = "Spell mastery — If there are two or more instant and/or sorcery cards in your graveyard, you may cast this spell as though it had flash. (You may cast it any time you could cast an instant.)\nDestroy target tapped creature.";
    let swift = engine::parser::oracle::parse_oracle_text(
        swift_text,
        "Swift Reckoning",
        &[],
        &strings(&["Sorcery"]),
        &[],
    );
    assert_eq!(
        swift.casting_options.len(),
        1,
        "{:?}",
        swift.casting_options
    );
    assert_eq!(
        swift.casting_options[0].kind,
        SpellCastingOptionKind::AsThoughHadFlash
    );
    assert_eq!(swift.casting_options[0].cost, None);
    assert!(
        matches!(
            swift.casting_options[0].condition,
            Some(ParsedCondition::QuantityComparison {
                lhs: QuantityExpr::Ref {
                    qty: QuantityRef::ZoneCardCount { .. },
                },
                comparator: Comparator::GE,
                rhs: QuantityExpr::Fixed { value: 2 },
            })
        ),
        "{:?}",
        swift.casting_options[0]
    );
    let swift_effects = all_effects(&swift.abilities);
    assert!(
        swift_effects
            .iter()
            .any(|e| matches!(e, Effect::Destroy { .. })),
        "{swift_effects:?}"
    );

    for effects in [all_effects(&grip.abilities), swift_effects] {
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::CastFromZone { .. })),
            "{effects:?}"
        );
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::Unimplemented { .. })),
            "{effects:?}"
        );
    }
}

/// One row of the unrepresentable self casting-option table: card name, verbatim Oracle
/// text, types, subtypes, MTGJSON keyword names, and the reach-guard over the effects.
type GapRow = (
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
    &'static [&'static str],
    fn(&engine::parser::oracle::ParsedAbilities) -> bool,
);

fn has_effect(
    parsed: &engine::parser::oracle::ParsedAbilities,
    predicate: impl Fn(&Effect) -> bool,
) -> bool {
    all_effects(&parsed.abilities).iter().any(predicate)
}

fn has_casting_option_gap_described_as_its_line(
    parsed: &engine::parser::oracle::ParsedAbilities,
) -> bool {
    parsed.abilities.iter().any(|ability| {
        matches!(
            &*ability.effect,
            Effect::Unimplemented { name, description: Some(description) }
                if name == "casting_option"
                    && ability.description.as_deref() == Some(description.as_str())
                    && description.to_lowercase().contains("you may cast")
        )
    })
}

const UNREPRESENTABLE_SELF_CASTING_OPTIONS: &[GapRow] = &[
    (
        "Summoning Trap",
        "If a creature spell you cast this turn was countered by a spell or ability an opponent controlled, you may pay {0} rather than pay this spell's mana cost.\nLook at the top seven cards of your library. You may put a creature card from among them onto the battlefield. Put the rest on the bottom of your library in any order.",
        &["Instant"],
        &["Trap"],
        &[],
        |parsed| has_effect(parsed, |e| matches!(e, Effect::Dig { .. })),
    ),
    (
        "Inferno Trap",
        "If you've been dealt damage by two or more creatures this turn, you may pay {R} rather than pay this spell's mana cost.\nInferno Trap deals 4 damage to target creature.",
        &["Instant"],
        &["Trap"],
        &[],
        |parsed| has_effect(parsed, |e| matches!(e, Effect::DealDamage { .. })),
    ),
    (
        "Cobra Trap",
        "If a noncreature permanent under your control was destroyed this turn by a spell or ability an opponent controlled, you may pay {G} rather than pay this spell's mana cost.\nCreate four 1/1 green Snake creature tokens.",
        &["Instant"],
        &["Trap"],
        &[],
        |parsed| has_effect(parsed, |e| matches!(e, Effect::Token { .. })),
    ),
    (
        "Erase (Not the Urza's Legacy One)",
        "If you control two or more white permanents that share an artist, you may cast this spell without paying its mana cost.\nExile target enchantment.",
        &["Instant"],
        &[],
        &[],
        |parsed| {
            has_effect(parsed, |e| {
                matches!(
                    e,
                    Effect::ChangeZone {
                        destination: engine::types::zones::Zone::Exile,
                        ..
                    }
                )
            })
        },
    ),
    (
        "Remodel",
        "If you control two or more green permanents that share an artist, you may cast this spell without paying its mana cost.\nExile target artifact.",
        &["Instant"],
        &[],
        &[],
        |parsed| has_effect(parsed, |e| matches!(e, Effect::ChangeZone { .. })),
    ),
    (
        "Silver Scrutiny",
        "You may cast this spell as though it had flash if X is 3 or less.\nDraw X cards.",
        &["Sorcery"],
        &[],
        &[],
        |parsed| has_effect(parsed, |e| matches!(e, Effect::Draw { .. })),
    ),
    (
        "Shove Aside",
        "If you aren't the starting player, you may cast Shove Aside as though it had flash.\nShove Aside deals 3 damage to target creature or planeswalker.",
        &["Sorcery"],
        &[],
        &[],
        |parsed| has_effect(parsed, |e| matches!(e, Effect::DealDamage { .. })),
    ),
    (
        "Qasali Ambusher",
        "Reach\nIf a creature is attacking you and you control a Forest and a Plains, you may cast this creature without paying its mana cost and as though it had flash.",
        &["Creature"],
        &["Cat", "Warrior"],
        &["Reach"],
        has_casting_option_gap_described_as_its_line,
    ),
    (
        "Quantum Reduction",
        "Teamwork 2 (As an additional cost to cast this spell, you may tap any number of creatures you control with total power 2 or more.)\nYou may cast this spell as though it had flash if it's cast using teamwork.\nEnchant creature\nEnchanted creature gets -5/-0 and loses all abilities.",
        &["Enchantment"],
        &["Aura"],
        &["Teamwork", "Enchant"],
        |parsed| has_casting_option_gap_described_as_its_line(parsed) && !parsed.statics.is_empty(),
    ),
];

/// CR 118.9 / CR 601.3 / CR 601.3d: a self casting-option line (verbatim) whose option
/// cannot be represented still only permits casting the spell. It must be an honest
/// coverage gap — never a resolution `PayCost` (paying the alternative cost on top of
/// the full mana cost) or a self-recast `CastFromZone` — while the card's own
/// instruction still parses.
#[test]
fn unrepresentable_self_casting_option_is_a_gap_not_a_resolution_effect() {
    for &(name, text, types, subtypes, keywords, reach_guard) in
        UNREPRESENTABLE_SELF_CASTING_OPTIONS
    {
        let parsed = engine::parser::oracle::parse_oracle_text(
            text,
            name,
            &strings(keywords),
            &strings(types),
            &strings(subtypes),
        );
        let effects = all_effects(&parsed.abilities);
        assert!(reach_guard(&parsed), "{name}: {parsed:?}");
        assert!(
            parsed.casting_options.is_empty(),
            "{name}: {:?}",
            parsed.casting_options
        );
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::PayCost { .. } | Effect::CastFromZone { .. })),
            "{name}: {effects:?}"
        );
        assert_eq!(
            effects
                .iter()
                .filter(|e| matches!(
                    e,
                    Effect::Unimplemented { name, .. } if name == "casting_option"
                ))
                .count(),
            1,
            "{name}: {effects:?}"
        );
        let face = engine::types::card::CardFace {
            name: name.to_string(),
            oracle_text: Some(text.to_string()),
            abilities: parsed.abilities,
            ..Default::default()
        };
        assert!(
            !engine::game::coverage::card_face_gaps(&face).is_empty(),
            "{name}"
        );
    }

    // Hostile sibling: Mindbreak Trap's condition is representable, so its line is a
    // real casting option and no gap.
    let mindbreak = engine::parser::oracle::parse_oracle_text(
        "If an opponent cast three or more spells this turn, you may pay {0} rather than pay this spell's mana cost.\nExile any number of target spells.",
        "Mindbreak Trap",
        &[],
        &strings(&["Instant"]),
        &strings(&["Trap"]),
    );
    assert_eq!(mindbreak.casting_options.len(), 1);
    assert!(
        !all_effects(&mindbreak.abilities)
            .iter()
            .any(|e| matches!(e, Effect::Unimplemented { .. })),
        "{:?}",
        mindbreak.abilities
    );
}

/// CR 118.9 / CR 601.3d: a rejected self casting-option line that follows a
/// resolution line ends the spell body instead of being absorbed as a resolution
/// instruction. Synthetic: Inferno Trap's two lines in reverse order. No printed card
/// puts a rejected option line second; the nearest printed shape is Graveyard Dig,
/// whose casting-option line follows its resolution line.
#[test]
fn rejected_casting_option_line_ends_the_spell_body() {
    let parsed = engine::parser::oracle::parse_oracle_text(
        "Inferno Trap deals 4 damage to target creature.\nIf you've been dealt damage by two or more creatures this turn, you may pay {R} rather than pay this spell's mana cost.",
        "Inferno Trap",
        &[],
        &strings(&["Instant"]),
        &strings(&["Trap"]),
    );
    assert_eq!(parsed.abilities.len(), 2, "{:?}", parsed.abilities);
    // Reach-guard: the resolution instruction is still parsed.
    assert!(
        matches!(*parsed.abilities[0].effect, Effect::DealDamage { .. }),
        "{:?}",
        parsed.abilities[0]
    );
    assert!(
        matches!(
            &*parsed.abilities[1].effect,
            Effect::Unimplemented { name, .. } if name == "casting_option"
        ),
        "{:?}",
        parsed.abilities[1]
    );
    let effects = all_effects(&parsed.abilities);
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::PayCost { .. })),
        "{effects:?}"
    );
}
