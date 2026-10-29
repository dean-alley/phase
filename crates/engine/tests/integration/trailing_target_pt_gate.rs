//! Trailing target power/toughness gates — "<targeted instruction> if its
//! power/toughness is <threshold>", where the threshold is fixed or dynamic.
//!
//! Before the fix the trailing gate was dropped when its threshold was dynamic
//! (Ghastly Demise, Desperate Plea, Titan's Presence, Sage-Eye Avengers), bound
//! to the trigger SOURCE when hoisted as an intervening "if" (The Hippodrome),
//! or read an absent cost object (Ent's Fury). Every row below drives the real
//! cast / trigger pipeline; each card's two gate sides are asserted here,
//! except Ent's Fury, whose gate-holds side is
//! issue_735_cost_paid_object_non_regression.rs.
//!
//! CR references (verified against docs/MagicCompRules.txt):
//!   - CR 608.2c: instructions are followed in order, applying the rules of
//!     English — "its" names the instruction's own announced target.
//!   - CR 115.1: targets are declared as the spell or ability is put on the
//!     stack.
//!   - CR 208.1: power and toughness.
//!   - CR 608.2h: the comparison reads current information once, when the
//!     effect is applied (or the source's last known information).
//!   - CR 608.2b: an illegal target is not acted on.
//!   - CR 608.2k: "the revealed card" / "the sacrificed creature" names the
//!     untargeted object the cost referred to.
//!   - CR 608.2n: an instant or sorcery goes to its owner's graveyard as the
//!     final part of its resolution, so it does not count itself.
//!   - CR 603.4: a post-effect "if" is not an intervening "if"; the trigger
//!     always goes on the stack and the gate is checked at resolution.
//!   - CR 311.7 / CR 901.9b: chaos abilities trigger on a plane when chaos
//!     ensues.

use engine::game::layers::evaluate_layers;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::game::triggers::process_triggers;
use engine::types::ability::TargetRef;
use engine::types::actions::GameAction;
use engine::types::events::GameEvent;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

use super::rules::AttackTarget;

const GHASTLY_DEMISE_ORACLE: &str = "Destroy target nonblack creature if its toughness is less than or equal to the number of cards in your graveyard.";

const ENTS_FURY_ORACLE: &str = "Put a +1/+1 counter on target creature you control if its power is 4 or greater. Then that creature gets +1/+1 until end of turn and fights target creature you don't control.";

/// Sage-Eye Avengers' second paragraph, verbatim. Prowess (the first
/// paragraph) is omitted: it triggers only on noncreature spells, none of
/// which these rows cast, so it cannot affect the attack trigger under test.
const SAGE_EYE_TRIGGER_ORACLE: &str = "Whenever this creature attacks, you may return target creature to its owner's hand if its power is less than this creature's power.";

const DESPERATE_PLEA_ORACLE: &str = "As an additional cost to cast this spell, sacrifice a creature.\nChoose one or both —\n• Return target creature card from your graveyard to the battlefield if its power is less than or equal to the sacrificed creature's power.\n• Destroy target creature.";

const TITANS_PRESENCE_ORACLE: &str = "As an additional cost to cast this spell, reveal a colorless creature card from your hand.\nExile target creature if its power is less than or equal to the revealed card's power.";

const HIPPODROME_ORACLE: &str = "All creatures get -5/-0.\nWhenever chaos ensues, you may destroy target creature if its power is 0 or less.";

fn zone_of(runner: &GameRunner, id: ObjectId) -> Zone {
    runner.state().objects[&id].zone
}

/// Completion boundary: the stack is empty and the game is back at priority.
fn assert_settled(runner: &GameRunner) {
    assert!(
        runner.state().stack.is_empty(),
        "stack must be empty, got {:?}",
        runner.state().stack
    );
    assert!(
        matches!(runner.state().waiting_for, WaitingFor::Priority { .. }),
        "expected Priority, got {:?}",
        runner.state().waiting_for
    );
}

/// What a trigger drive observed.
struct TriggerDrive {
    target_prompted: bool,
    optional_offered: bool,
}

/// Drive a pending trigger to an empty stack: order triggers, answer the
/// target prompt with `target`, accept any optional ("you may") prompt, and
/// pass priority until the stack empties.
fn drive_trigger(runner: &mut GameRunner, target: ObjectId) -> TriggerDrive {
    let mut drive = TriggerDrive {
        target_prompted: false,
        optional_offered: false,
    };
    for _ in 0..64 {
        match runner.state().waiting_for.clone() {
            WaitingFor::OrderTriggers { triggers, .. } => {
                let order = (0..triggers.len()).collect();
                runner
                    .act(GameAction::OrderTriggers { order })
                    .expect("ordering triggers must succeed");
            }
            WaitingFor::TriggerTargetSelection { .. } => {
                assert!(!drive.target_prompted, "the target prompt appears once");
                drive.target_prompted = true;
                runner
                    .act(GameAction::ChooseTarget {
                        target: Some(TargetRef::Object(target)),
                    })
                    .expect("choosing the trigger target must succeed");
            }
            WaitingFor::OptionalEffectChoice { .. } => {
                drive.optional_offered = true;
                runner
                    .act(GameAction::DecideOptionalEffect { accept: true })
                    .expect("accepting the optional effect must succeed");
            }
            WaitingFor::Priority { .. } => {
                if runner.state().stack.is_empty() {
                    return drive;
                }
                runner
                    .act(GameAction::PassPriority)
                    .expect("passing priority must succeed");
            }
            other => panic!("unexpected prompt while driving the trigger: {other:?}"),
        }
    }
    panic!("the trigger did not settle within the prompt budget");
}

// ---------------------------------------------------------------------------
// Ghastly Demise — dynamic threshold: the caster's graveyard size.
// ---------------------------------------------------------------------------

/// Builds Ghastly Demise against a 2/3 victim, with `own_graveyard` creature
/// cards in P0's graveyard and `opponent_graveyard` in P1's.
fn ghastly_demise_board(
    own_graveyard: usize,
    opponent_graveyard: usize,
) -> (GameRunner, ObjectId, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Ghastly Demise", true, GHASTLY_DEMISE_ORACLE)
        .with_mana_cost(ManaCost::zero())
        .id();
    let victim = scenario.add_creature(P1, "Victim", 2, 3).id();
    for i in 0..own_graveyard {
        scenario.add_creature_to_graveyard(P0, &format!("Own Corpse {i}"), 1, 1);
    }
    for i in 0..opponent_graveyard {
        scenario.add_creature_to_graveyard(P1, &format!("Opponent Corpse {i}"), 1, 1);
    }
    (scenario.build(), spell, victim)
}

/// CR 608.2c + CR 208.1: toughness 3 ≤ three cards in the caster's graveyard —
/// the victim is destroyed.
#[test]
fn ghastly_demise_destroys_when_toughness_at_most_your_graveyard_size() {
    let (mut runner, spell, victim) = ghastly_demise_board(3, 0);
    let outcome = runner.cast(spell).target_objects(&[victim]).resolve();
    outcome.assert_zone(&[victim], Zone::Graveyard);
    assert!(outcome.state().stack.is_empty());
    assert!(matches!(
        outcome.final_waiting_for(),
        WaitingFor::Priority { .. }
    ));
}

/// CR 608.2c + CR 208.1 + CR 608.2n: toughness 3 > two cards in the caster's
/// graveyard — the victim survives. The opponent's larger graveyard (5) proves
/// "your graveyard" is the caster's. Reverting the fix drops the gate and the
/// victim is destroyed.
#[test]
fn ghastly_demise_spares_when_toughness_exceeds_your_graveyard_size() {
    let (mut runner, spell, victim) = ghastly_demise_board(2, 5);
    let outcome = runner.cast(spell).target_objects(&[victim]).resolve();
    outcome.assert_zone(&[victim], Zone::Battlefield);
    // Reach guard: the spell resolved (CR 608.2n) rather than stalling.
    outcome.assert_zone(&[spell], Zone::Graveyard);
    assert!(outcome.state().stack.is_empty());
    assert!(matches!(
        outcome.final_waiting_for(),
        WaitingFor::Priority { .. }
    ));
}

// ---------------------------------------------------------------------------
// Ent's Fury — fixed threshold, two targets: the gate reads the first.
// ---------------------------------------------------------------------------

/// CR 608.2c + CR 115.1: "its" is the counter instruction's target (slot 0,
/// the 3/6 ally), not the fight target (slot 1, a 5/1). Power 3 < 4, so no
/// counter; the unconditional pump still applies (3 → 4). Reading slot 1
/// (power 5) would add the counter (power 5).
#[test]
fn ents_fury_gate_reads_the_counter_target_not_the_fight_target() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let ally = scenario.add_creature(P0, "Ally", 3, 6).id();
    let enemy = scenario.add_creature(P1, "Enemy", 5, 1).id();
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Ent's Fury", false, ENTS_FURY_ORACLE)
        .with_mana_cost(ManaCost::zero())
        .id();
    let mut runner = scenario.build();

    let outcome = runner.cast(spell).target_objects(&[ally, enemy]).resolve();

    assert_eq!(
        outcome.state().objects[&ally].power,
        Some(4),
        "no counter (power 3 < 4); the +1/+1 pump applies"
    );
    // Reach guard: the fight ran after the gated node.
    outcome.assert_zone(&[enemy], Zone::Graveyard);
    outcome.assert_zone(&[ally], Zone::Battlefield);
    assert!(outcome.state().stack.is_empty());
    assert!(matches!(
        outcome.final_waiting_for(),
        WaitingFor::Priority { .. }
    ));
}

// ---------------------------------------------------------------------------
// Sage-Eye Avengers — trigger, dynamic threshold: the source's power.
// ---------------------------------------------------------------------------

fn sage_eye_attack(target_power: i32) -> (GameRunner, ObjectId, TriggerDrive) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let sage = scenario
        .add_creature_from_oracle(P0, "Sage-Eye Avengers", 4, 5, SAGE_EYE_TRIGGER_ORACLE)
        .id();
    let target = scenario
        .add_creature(P1, "Target", target_power, target_power)
        .id();
    let mut runner = scenario.build();

    runner.advance_to_combat();
    runner
        .declare_attackers(&[(sage, AttackTarget::Player(P1))])
        .expect("declaring Sage-Eye Avengers as an attacker must succeed");
    let drive = drive_trigger(&mut runner, target);
    (runner, target, drive)
}

/// CR 608.2c + CR 608.2h: target power 3 < Sage-Eye's power 4 — the target
/// returns to its owner's hand. Binding "its" to the attacking source would
/// compare 4 < 4 and never return it.
#[test]
fn sage_eye_avengers_returns_target_with_less_power() {
    let (runner, target, drive) = sage_eye_attack(3);
    assert!(
        drive.target_prompted,
        "the attack trigger asked for a target"
    );
    assert!(
        drive.optional_offered,
        "the gate held, so the optional prompt appears"
    );
    assert_eq!(zone_of(&runner, target), Zone::Hand);
    assert_eq!(runner.state().objects[&target].owner, P1);
    assert_settled(&runner);
}

/// CR 608.2c + CR 603.4: target power 4 is not less than Sage-Eye's power 4 —
/// the gate fails at resolution, before the optional prompt, so the target
/// stays. Reverting the fix drops the gate: the prompt appears, is accepted,
/// and the target is returned.
#[test]
fn sage_eye_avengers_keeps_target_with_equal_power() {
    let (runner, target, drive) = sage_eye_attack(4);
    assert!(
        drive.target_prompted,
        "the attack trigger reached the stack"
    );
    assert!(
        !drive.optional_offered,
        "the failed gate is checked before the optional prompt"
    );
    assert_eq!(zone_of(&runner, target), Zone::Battlefield);
    assert_settled(&runner);
}

// ---------------------------------------------------------------------------
// Desperate Plea — modal, dynamic threshold: the sacrificed creature's power.
// ---------------------------------------------------------------------------

/// Casts Desperate Plea choosing only its first mode, targeting a graveyard
/// creature of `corpse_power`, and sacrificing a 3/3.
fn desperate_plea_return(corpse_power: i32) -> (Zone, Zone, Zone, bool) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Desperate Plea", false, DESPERATE_PLEA_ORACLE)
        .with_mana_cost(ManaCost::zero())
        .id();
    let corpse = scenario
        .add_creature_to_graveyard(P0, "Corpse", corpse_power, corpse_power)
        .id();
    let fodder = scenario.add_creature(P0, "Fodder", 3, 3).id();
    let bystander = scenario.add_creature(P0, "Bystander", 6, 6).id();
    let mut runner = scenario.build();

    let outcome = runner
        .cast(spell)
        .modes(&[0])
        .target_objects(&[corpse])
        .sacrifice_with(&[fodder])
        .resolve();
    let settled = outcome.state().stack.is_empty()
        && matches!(outcome.final_waiting_for(), WaitingFor::Priority { .. });
    (
        outcome.zone_of(corpse),
        outcome.zone_of(fodder),
        outcome.zone_of(bystander),
        settled,
    )
}

/// CR 608.2k + CR 208.1: the corpse's power 3 ≤ the sacrificed Fodder's
/// power 3 — the corpse returns to the battlefield.
#[test]
fn desperate_plea_returns_creature_with_power_at_most_the_sacrificed_one() {
    let (corpse, fodder, bystander, settled) = desperate_plea_return(3);
    assert_eq!(corpse, Zone::Battlefield);
    assert_eq!(fodder, Zone::Graveyard, "the sacrifice cost was paid");
    assert_eq!(bystander, Zone::Battlefield);
    assert!(settled);
}

/// CR 608.2k + CR 208.1: the corpse's power 4 > the sacrificed Fodder's power
/// 3 — the corpse stays in the graveyard. Reverting the fix drops the gate and
/// the corpse returns.
#[test]
fn desperate_plea_keeps_creature_with_power_above_the_sacrificed_one() {
    let (corpse, fodder, bystander, settled) = desperate_plea_return(4);
    assert_eq!(corpse, Zone::Graveyard);
    assert_eq!(fodder, Zone::Graveyard, "the sacrifice cost was paid");
    assert_eq!(bystander, Zone::Battlefield);
    assert!(settled);
}

// ---------------------------------------------------------------------------
// Titan's Presence — dynamic threshold: the revealed card's power.
// ---------------------------------------------------------------------------

/// Casts Titan's Presence, revealing a colorless 4/4 creature card, against a
/// target of `target_power`.
fn titans_presence_exile(target_power: i32) -> (Zone, Zone, bool) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Titan's Presence", true, TITANS_PRESENCE_ORACLE)
        .with_mana_cost(ManaCost::zero())
        .id();
    let revealed = scenario
        .add_creature_to_hand(P0, "Colorless Titan", 4, 4)
        .with_mana_cost(ManaCost::generic(4))
        .id();
    let target = scenario
        .add_creature(P1, "Target", target_power, target_power)
        .id();
    let mut runner = scenario.build();

    let outcome = runner
        .cast(spell)
        .target_objects(&[target])
        .pay_cost_with(&[revealed])
        .resolve();
    let settled = outcome.state().stack.is_empty()
        && matches!(outcome.final_waiting_for(), WaitingFor::Priority { .. });
    (outcome.zone_of(target), outcome.zone_of(revealed), settled)
}

/// CR 608.2k + CR 208.1: target power 4 ≤ the revealed card's power 4 — the
/// target is exiled.
#[test]
fn titans_presence_exiles_target_with_power_at_most_the_revealed_card() {
    let (target, revealed, settled) = titans_presence_exile(4);
    assert_eq!(target, Zone::Exile);
    assert_eq!(revealed, Zone::Hand, "revealing keeps the card in hand");
    assert!(settled);
}

/// CR 608.2k + CR 208.1: target power 5 > the revealed card's power 4 — the
/// target stays. Reverting the fix drops the gate and the target is exiled.
#[test]
fn titans_presence_spares_target_with_power_above_the_revealed_card() {
    let (target, revealed, settled) = titans_presence_exile(5);
    assert_eq!(target, Zone::Battlefield);
    assert_eq!(revealed, Zone::Hand, "revealing keeps the card in hand");
    assert!(settled);
}

// ---------------------------------------------------------------------------
// The Hippodrome — chaos trigger, fixed threshold on the target's CURRENT power.
// ---------------------------------------------------------------------------

/// Places The Hippodrome (all creatures get −5/−0) with P1's `target` creature
/// and P0's `bystander`, fires chaos, and drives the trigger choosing `target`.
fn hippodrome_chaos(
    target_size: i32,
    bystander_size: i32,
) -> (GameRunner, ObjectId, ObjectId, TriggerDrive) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let hippodrome = scenario
        .add_land_from_oracle(P0, "The Hippodrome", HIPPODROME_ORACLE)
        .id();
    let target = scenario
        .add_creature(P1, "Target", target_size, target_size)
        .id();
    let bystander = scenario
        .add_creature(P0, "Bystander", bystander_size, bystander_size)
        .id();
    let mut runner = scenario.build();
    evaluate_layers(runner.state_mut());
    assert_eq!(
        runner.state().objects[&target].power,
        Some(target_size - 5),
        "precondition: the plane's static lowers the target's power"
    );

    // CR 311.7 / CR 901.9b: the chaos event is keyed by the plane whose ability
    // fires; synthesizing it exercises the real plane-identity binding without
    // planar-deck setup (precedent: selesnya_loft_gardens_mixed_mana_choice.rs).
    process_triggers(
        runner.state_mut(),
        &[GameEvent::ChaosEnsued {
            plane_id: hippodrome,
        }],
    );
    let drive = drive_trigger(&mut runner, target);
    (runner, target, bystander, drive)
}

/// CR 608.2c + CR 608.2h: a 3/3's current power is −2 ≤ 0 — the target is
/// destroyed.
#[test]
fn the_hippodrome_destroys_target_with_current_power_at_most_zero() {
    let (runner, target, bystander, drive) = hippodrome_chaos(3, 6);
    assert!(
        drive.target_prompted,
        "the chaos trigger asked for a target"
    );
    assert!(
        drive.optional_offered,
        "the gate held, so the optional prompt appears"
    );
    assert_eq!(zone_of(&runner, target), Zone::Graveyard);
    assert_eq!(zone_of(&runner, bystander), Zone::Battlefield);
    assert_settled(&runner);
}

/// CR 608.2c + CR 603.4: a 6/6's current power is 1 > 0 — the target survives.
/// Reverting the fix hoists the gate onto the trigger with "its" bound to the
/// plane (power 0 ≤ 0, always true): the prompt appears, is accepted, and the
/// target is destroyed. The 3/3 bystander (current power −2) is not chosen.
#[test]
fn the_hippodrome_spares_target_with_current_power_above_zero() {
    let (runner, target, bystander, drive) = hippodrome_chaos(6, 3);
    assert!(drive.target_prompted, "the chaos trigger reached the stack");
    assert!(
        !drive.optional_offered,
        "the failed gate is checked before the optional prompt"
    );
    assert_eq!(zone_of(&runner, target), Zone::Battlefield);
    assert_eq!(zone_of(&runner, bystander), Zone::Battlefield);
    assert_settled(&runner);
}
