//! Non-regression for issue #735 (Edit B) — cards whose "if its power is …"
//! clause has a TARGET or ENTERING-REFERENT subject must not bind "its" to the
//! ability source. Edit B only flips player/phase-subject triggers (Amalia,
//! Lily) to `Source`; "its" still binds the clause-local object (the entering
//! creature / the target).
//!
//! Cards covered:
//!   - Tribute to the World Tree: "Whenever a creature you control enters, draw
//!     a card if its power is 3 or greater. …" — "its" = the ENTERING creature
//!     (`ObjectScope::CostPaidObject`).
//!   - Ent's Fury: "Put a +1/+1 counter on target creature you control if its
//!     power is 4 or greater. …" — "its" = the counter instruction's announced
//!     TARGET (CR 608.2c + CR 115.1), read live by a `TargetMatchesFilter`
//!     gate.
//!
//! CR references (verified against docs/MagicCompRules.txt):
//!   - CR 608.2h: the power comparison reads the referenced object's current
//!     info once, when the effect is applied.
//!   - CR 608.2k: an effect referring to an untargeted object previously named
//!     by the ability (the entering/cost-paid object) still affects that object.
//!   - CR 608.2c + CR 115.1: "its" after a targeted instruction names that
//!     instruction's announced target (rules of English).

use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::game::triggers::{drain_order_triggers_with_identity, process_triggers};
use engine::game::zones::{create_object, move_to_zone};
use engine::types::card_type::CoreType;
use engine::types::counter::CounterType;
use engine::types::identifiers::{CardId, ObjectId};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const TRIBUTE_ORACLE: &str = "Whenever a creature you control enters, draw a card if its power is 3 or greater. Otherwise, put two +1/+1 counters on it.";

const ENTS_FURY_ORACLE: &str = "Put a +1/+1 counter on target creature you control if its power is 4 or greater. Then that creature gets +1/+1 until end of turn and fights target creature you don't control.";

fn hand_size(runner: &GameRunner, player: engine::types::player::PlayerId) -> usize {
    runner.state().players[player.0 as usize].hand.len()
}

fn p1p1(runner: &GameRunner, id: ObjectId) -> u32 {
    runner.state().objects[&id]
        .counters
        .get(&CounterType::Plus1Plus1)
        .copied()
        .unwrap_or(0)
}

/// Create a creature in `owner`'s hand with the given power and move it to the
/// battlefield, firing "creature you control enters" triggers.
fn enter_creature(runner: &mut GameRunner, owner: engine::types::player::PlayerId, power: i32) {
    let cid = {
        let state = runner.state_mut();
        let card_id = CardId(state.next_object_id);
        let id = create_object(state, card_id, owner, "Bystander".to_string(), Zone::Hand);
        let obj = state.objects.get_mut(&id).unwrap();
        obj.card_types.core_types.push(CoreType::Creature);
        obj.base_card_types = obj.card_types.clone();
        obj.power = Some(power);
        obj.toughness = Some(power.max(1));
        obj.base_power = Some(power);
        obj.base_toughness = Some(power.max(1));
        id
    };
    let mut events = Vec::new();
    move_to_zone(runner.state_mut(), cid, Zone::Battlefield, &mut events);
    process_triggers(runner.state_mut(), &events);
    drain_order_triggers_with_identity(runner.state_mut());
    runner.advance_until_stack_empty();
}

fn setup_tribute() -> GameRunner {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let mut builder = scenario.add_creature(P0, "Tribute to the World Tree", 0, 0);
    builder.as_enchantment();
    builder.from_oracle_text(TRIBUTE_ORACLE);
    // Library padding so a draw does not deck P0.
    for _ in 0..10 {
        scenario.add_card_to_library_top(P0, "Forest");
    }
    scenario.build()
}

/// NON-REGRESSION (Tribute): an entering creature with power ≥ 3 draws a card —
/// "its power" binds the ENTERING creature (CostPaidObject), so a 3-power
/// creature satisfies `Power{CostPaidObject} GE 3` and P0 draws.
#[test]
fn tribute_entering_power_3_draws_a_card() {
    let mut runner = setup_tribute();
    let before = hand_size(&runner, P0);

    enter_creature(&mut runner, P0, 3);

    assert_eq!(
        hand_size(&runner, P0),
        before + 1,
        "an entering 3-power creature must make Tribute draw a card (its = entering creature)"
    );
}

/// NON-REGRESSION (Tribute): an entering creature with power < 3 does NOT draw;
/// the else branch puts two +1/+1 counters on the entering creature instead.
#[test]
fn tribute_entering_power_2_puts_counters_not_draw() {
    let mut runner = setup_tribute();
    let before = hand_size(&runner, P0);

    enter_creature(&mut runner, P0, 2);

    assert_eq!(
        hand_size(&runner, P0),
        before,
        "an entering 2-power creature must NOT draw (its power 2 < 3)"
    );
    // The entering creature ("Bystander") received two +1/+1 counters.
    let bystander = runner
        .state()
        .battlefield
        .iter()
        .find(|id| runner.state().objects[id].name == "Bystander")
        .copied()
        .expect("the entering creature is on the battlefield");
    assert_eq!(
        p1p1(&runner, bystander),
        2,
        "the else branch must put two +1/+1 counters on the entering creature"
    );
}

/// CR 608.2c + CR 115.1 + CR 208.1 (Ent's Fury): "its" names the counter
/// instruction's announced target, a 4/4 ally. Its power 4 ≥ 4, so it gets the
/// +1/+1 counter (5/5), then the unconditional +1/+1 pump (6/6), and survives
/// fighting a 3/3. Before the fix the gate read `Power{CostPaidObject}` —
/// Ent's Fury pays no cost that supplies a cost object, so the gate was always
/// false and the ally ended at 5.
#[test]
fn ents_fury_counter_gate_reads_its_target() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    // P0's own creature (the counter/pump/fight subject) at power 4.
    let ally = scenario.add_creature(P0, "Strong Ally", 4, 4).id();
    // An opponent creature to satisfy "fights target creature you don't control".
    let enemy = scenario.add_creature(P1, "Enemy", 3, 3).id();
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Ent's Fury", false, ENTS_FURY_ORACLE)
        .with_mana_cost(engine::types::mana::ManaCost::zero())
        .id();

    let mut runner = scenario.build();
    engine::game::layers::evaluate_layers(runner.state_mut());
    assert_eq!(
        runner.state().objects[&ally].power,
        Some(4),
        "precondition: the ally target has power 4"
    );

    // Two target slots: the subject (a creature you control) and the fight
    // opponent (a creature you don't control).
    let outcome = runner.cast(spell).target_objects(&[ally, enemy]).resolve();

    // The gate holds on the target (power 4 ≥ 4): counter (4 → 5), then the
    // unconditional +1/+1 pump (5 → 6).
    assert_eq!(
        outcome.state().objects[&ally].power,
        Some(6),
        "Ent's Fury's counter and +1/+1 pump must both apply to the target (4 -> 6)"
    );
    assert_eq!(
        outcome.state().objects[&ally]
            .counters
            .get(&CounterType::Plus1Plus1)
            .copied()
            .unwrap_or(0),
        1,
        "the gated counter must be placed on the target"
    );
    // The target survives the fight (6/6 vs a 3/3 dealing 3).
    assert_eq!(
        outcome.state().objects[&ally].zone,
        Zone::Battlefield,
        "the 6/6 target survives fighting a 3/3"
    );
}
