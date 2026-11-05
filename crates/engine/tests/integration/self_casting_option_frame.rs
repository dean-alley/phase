//! Self casting-option lines behind an ability word (CR 207.2c), driven through
//! the real cast pipeline.
//!
//! Verbatim Oracle text (MTGJSON):
//! - Admiral's Order {1}{U}{U}, Instant: "Raid — If you attacked this turn, you
//!   may pay {U} rather than pay this spell's mana cost.\nCounter target spell."
//! - Swift Reckoning {1}{W}, Sorcery: "Spell mastery — If there are two or more
//!   instant and/or sorcery cards in your graveyard, you may cast this spell as
//!   though it had flash. (You may cast it any time you could cast an
//!   instant.)\nDestroy target tapped creature."
//! - Dragon Grip {2}{R}, Enchantment — Aura: "Ferocious — If you control a
//!   creature with power 4 or greater, you may cast this spell as though it had
//!   flash. (You may cast it any time you could cast an instant.)\nEnchant
//!   creature\nEnchanted creature gets +2/+0 and has first strike."
//!
//! The ability words "Raid", "Spell mastery" and "Ferocious" have no rules
//! meaning, so each labeled line is a casting option: a raid-gated {U}
//! alternative cost (CR 118.9) or a conditional flash permission (CR 601.3d).
//! Each condition is read for the casting player (CR 109.5), so the
//! multi-authority rows pin that an opponent's attack, graveyard, or creature
//! does not satisfy it.

use engine::game::combat::AttackTarget;
use engine::game::engine::EngineError;
use engine::game::game_object::AttachTarget;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::actions::GameAction;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard};
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;

const ADMIRALS_ORDER: &str = "Raid — If you attacked this turn, you may pay {U} rather than pay this spell's mana cost.\nCounter target spell.";

const SWIFT_RECKONING: &str = "Spell mastery — If there are two or more instant and/or sorcery cards in your graveyard, you may cast this spell as though it had flash. (You may cast it any time you could cast an instant.)\nDestroy target tapped creature.";

const DRAGON_GRIP: &str = "Ferocious — If you control a creature with power 4 or greater, you may cast this spell as though it had flash. (You may cast it any time you could cast an instant.)\nEnchant creature\nEnchanted creature gets +2/+0 and has first strike.";

/// The rejection reason for a noninstant spell cast outside its controller's
/// main-phase, empty-stack window (CR 117.1a).
const SORCERY_TIMING_REJECTION: &str = "Sorcery-speed spells can only be cast by the active player";

// ---------------------------------------------------------------------------
// Admiral's Order
// ---------------------------------------------------------------------------

struct OrderBoard {
    runner: GameRunner,
    order: ObjectId,
    /// P0's own zero-cost instant, cast first so Admiral's Order has a spell
    /// to target. Synthetic fixture setup; the card under test is real.
    victim: ObjectId,
    islands: Vec<ObjectId>,
}

/// Admiral's Order ({1}{U}{U}) and a zero-cost instant in P0's hand, `islands`
/// Islands on P0's battlefield, and one 2/2 for `attacker_owner`.
fn order_setup(
    attacker_owner: PlayerId,
    islands: usize,
) -> (GameScenario, ObjectId, ObjectId, ObjectId, Vec<ObjectId>) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let attacker = scenario.add_creature(attacker_owner, "Attacker", 2, 2).id();
    let island_ids = (0..islands)
        .map(|_| scenario.add_basic_land(P0, ManaColor::Blue))
        .collect();
    let order = scenario
        .add_spell_to_hand_from_oracle(P0, "Admiral's Order", true, ADMIRALS_ORDER)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Blue, ManaCostShard::Blue],
            generic: 1,
        })
        .id();
    let victim = scenario
        .add_spell_to_hand_from_oracle(P0, "Counterable Test Spell", true, "Draw a card.")
        .with_mana_cost(ManaCost::zero())
        .id();
    scenario.with_library_top(P0, &["Forest", "Forest", "Forest"]);
    scenario.with_library_top(P1, &["Forest", "Forest", "Forest"]);
    (scenario, order, victim, attacker, island_ids)
}

/// P0 attacks P1 with its own creature and keeps priority in the
/// declare-attackers step (CR 508.1a, CR 508.2).
fn order_own_attack(islands: usize) -> OrderBoard {
    let (scenario, order, victim, attacker, islands) = order_setup(P0, islands);
    let mut runner = scenario.build();
    runner.advance_to_combat();
    runner
        .declare_attackers(&[(attacker, AttackTarget::Player(P1))])
        .expect("P0 declares its attacker");
    assert_eq!(runner.state().phase, Phase::DeclareAttackers);
    assert_eq!(
        runner.state().waiting_for,
        WaitingFor::Priority { player: P0 }
    );
    OrderBoard {
        runner,
        order,
        victim,
        islands,
    }
}

/// No creature attacks: P0 holds priority in its own pre-combat main phase.
fn order_no_attack(islands: usize) -> OrderBoard {
    let (scenario, order, victim, _attacker, islands) = order_setup(P0, islands);
    let runner = scenario.build();
    OrderBoard {
        runner,
        order,
        victim,
        islands,
    }
}

/// P1 attacks P0: P1 is the active player, declares its attacker at P0, then
/// passes priority so P0 holds priority in the declare-attackers step.
fn order_opponent_attacks(islands: usize) -> OrderBoard {
    let (scenario, order, victim, attacker, islands) = order_setup(P1, islands);
    let mut runner = scenario.build();
    {
        let state = runner.state_mut();
        state.active_player = P1;
        state.priority_player = P1;
        state.waiting_for = WaitingFor::Priority { player: P1 };
    }
    runner.advance_to_combat();
    runner
        .declare_attackers(&[(attacker, AttackTarget::Player(P0))])
        .expect("P1 declares its attacker");
    runner
        .act(GameAction::PassPriority)
        .expect("P1 passes priority");
    assert_eq!(
        runner.state().waiting_for,
        WaitingFor::Priority { player: P0 }
    );
    assert!(runner.state().stack.is_empty());
    OrderBoard {
        runner,
        order,
        victim,
        islands,
    }
}

fn tapped_islands(board: &OrderBoard) -> usize {
    board
        .islands
        .iter()
        .filter(|island| board.runner.state().objects[*island].tapped)
        .count()
}

/// Cast the victim, then cast Admiral's Order targeting it and drive Admiral's
/// Order through resolution. Returns the cast's result and the waiting state
/// the pipeline halted in.
fn cast_order_at_victim(board: &mut OrderBoard) -> Result<WaitingFor, EngineError> {
    let (order, victim) = (board.order, board.victim);
    let mut committed = board.runner.cast(victim).commit();
    committed
        .cast(order)
        .target_objects(&[victim])
        .accept_optional()
        .try_resolve()
        .map(|outcome| outcome.final_waiting_for().clone())
}

/// PRIMARY: P0 attacked this turn, so the {U} alternative cost is offered and
/// Admiral's Order is cast from a single Island. Reverting the label read leaves
/// the line a resolution `PayCost` with no alternative cost, so {1}{U}{U} is
/// unpayable from one Island and the cast is rejected.
#[test]
fn admirals_order_raid_offers_u_after_you_attack() {
    let mut board = order_own_attack(1);
    let waiting_for =
        cast_order_at_victim(&mut board).expect("the raid alternative cost makes the cast payable");
    // CR 701.6a: the countered spell is put into its owner's graveyard.
    assert_eq!(
        board.runner.state().objects[&board.victim].zone,
        Zone::Graveyard
    );
    assert_eq!(
        board.runner.state().objects[&board.order].zone,
        Zone::Graveyard
    );
    // CR 118.9 + CR 118.9a: {U} is paid rather than the {1}{U}{U} mana cost.
    assert_eq!(tapped_islands(&board), 1);
    assert!(board.runner.state().stack.is_empty());
    // CR 117.3b: the active player receives priority after the spell resolves.
    assert_eq!(waiting_for, WaitingFor::Priority { player: P0 });
}

/// The gate is not unconditional: with no attack the alternative cost is not
/// offered and the full {1}{U}{U} is paid.
#[test]
fn admirals_order_no_attack_pays_full_cost() {
    let mut board = order_no_attack(3);
    let waiting_for =
        cast_order_at_victim(&mut board).expect("three Islands pay the full mana cost");
    // Positive reach-guard: the counter resolved.
    assert_eq!(
        board.runner.state().objects[&board.victim].zone,
        Zone::Graveyard
    );
    // CR 601.2f: with no alternative cost, the total cost is the mana cost.
    assert_eq!(tapped_islands(&board), 3);
    assert!(board.runner.state().stack.is_empty());
    assert_eq!(waiting_for, WaitingFor::Priority { player: P0 });
}

/// With no attack and only {U} available, Admiral's Order cannot be cast.
/// Reach-guard: `admirals_order_raid_offers_u_after_you_attack` casts from the
/// same single Island once P0 has attacked.
#[test]
fn admirals_order_no_attack_one_island_cannot_cast() {
    let mut board = order_no_attack(1);
    let result = cast_order_at_victim(&mut board);
    // CR 601.2f: {1}{U}{U} is unpayable from one Island.
    assert!(
        result.is_err(),
        "no attack: the {{U}} alternative cost is not offered, got {result:?}"
    );
    assert_eq!(
        board.runner.state().objects[&board.victim].zone,
        Zone::Stack
    );
    assert_eq!(board.runner.state().objects[&board.order].zone, Zone::Hand);
}

/// Multi-authority: an opponent attacking P0 does not satisfy "you attacked
/// this turn" (CR 109.5: "you" is Admiral's Order's would-be controller, P0).
/// Reach-guard: `admirals_order_raid_offers_u_after_you_attack`.
#[test]
fn admirals_order_opponent_attack_is_not_raid() {
    let mut board = order_opponent_attacks(1);
    let result = cast_order_at_victim(&mut board);
    match &result {
        Err(EngineError::ActionNotAllowed(msg)) if msg.contains("Cannot pay mana cost") => {}
        other => panic!("expected an unpayable {{1}}{{U}}{{U}} rejection, got {other:?}"),
    }
    assert_eq!(
        board.runner.state().objects[&board.victim].zone,
        Zone::Stack
    );
    assert_eq!(board.runner.state().objects[&board.order].zone, Zone::Hand);
}

// ---------------------------------------------------------------------------
// Swift Reckoning
// ---------------------------------------------------------------------------

/// Swift Reckoning ({1}{W}) in P0's hand with two Plains, a tapped P1 creature
/// to target, `own_graveyard` instants in P0's graveyard and `opponent_graveyard`
/// instants in P1's. P1 is the active player in its main phase and P0 holds
/// priority with an empty stack, so only a flash permission lets P0 cast it.
fn swift_reckoning_board(
    own_graveyard: usize,
    opponent_graveyard: usize,
) -> (GameRunner, ObjectId, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let victim = scenario.add_creature(P1, "Tapped Victim", 2, 2).id();
    for i in 0..own_graveyard {
        scenario.add_spell_to_graveyard(P0, &format!("Own Instant {i}"), true);
    }
    for i in 0..opponent_graveyard {
        scenario.add_spell_to_graveyard(P1, &format!("Opponent Instant {i}"), true);
    }
    scenario.add_basic_land(P0, ManaColor::White);
    scenario.add_basic_land(P0, ManaColor::White);
    let reckoning = scenario
        .add_spell_to_hand_from_oracle(P0, "Swift Reckoning", false, SWIFT_RECKONING)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::White],
            generic: 1,
        })
        .id();
    let mut runner = scenario.build();
    runner
        .state_mut()
        .objects
        .get_mut(&victim)
        .expect("victim exists")
        .tapped = true;
    {
        let state = runner.state_mut();
        state.active_player = P1;
        state.priority_player = P0;
        state.waiting_for = WaitingFor::Priority { player: P0 };
    }
    assert!(runner.state().stack.is_empty());
    (runner, reckoning, victim)
}

fn assert_sorcery_timing_rejection<T>(result: &Result<T, EngineError>) {
    match result {
        Err(EngineError::ActionNotAllowed(msg)) if msg.contains(SORCERY_TIMING_REJECTION) => {}
        Err(other) => panic!("expected a sorcery-timing rejection, got {other:?}"),
        Ok(_) => panic!("expected a sorcery-timing rejection, but the cast resolved"),
    }
}

/// PRIMARY: with two instant/sorcery cards in P0's graveyard, spell mastery lets
/// P0 cast Swift Reckoning during P1's turn (CR 601.3d + CR 207.2c). Reverting
/// the label read drops the flash permission and the cast is rejected.
#[test]
fn swift_reckoning_spell_mastery_grants_flash() {
    let (mut runner, reckoning, victim) = swift_reckoning_board(2, 0);
    let outcome = runner.cast(reckoning).target_object(victim).resolve();
    // CR 701.8a: the destroyed creature goes to its owner's graveyard.
    outcome.assert_zone(&[victim, reckoning], Zone::Graveyard);
    // CR 117.3b: the active player receives priority after the spell resolves.
    assert_eq!(
        outcome.final_waiting_for(),
        &WaitingFor::Priority { player: P1 }
    );
    assert!(runner.state().stack.is_empty());
}

/// One instant in P0's graveyard does not meet spell mastery, so Swift
/// Reckoning keeps sorcery timing (CR 117.1a). Reach-guard: the positive row.
#[test]
fn swift_reckoning_one_spell_in_graveyard_cannot_cast_at_instant_speed() {
    let (mut runner, reckoning, victim) = swift_reckoning_board(1, 0);
    let result = runner.cast(reckoning).target_object(victim).try_resolve();
    assert_sorcery_timing_rejection(&result);
    drop(result);
    assert_eq!(runner.state().objects[&reckoning].zone, Zone::Hand);
    assert!(runner.state().stack.is_empty());
}

/// Multi-authority: CR 601.3d grants flash only if its condition is met, and on
/// a spell being cast "your" is its would-be controller (CR 109.5), so instants
/// in P1's graveyard do not count toward P0's spell mastery. Reach-guard: the
/// positive row.
#[test]
fn swift_reckoning_opponents_graveyard_does_not_count() {
    let (mut runner, reckoning, victim) = swift_reckoning_board(1, 3);
    let result = runner.cast(reckoning).target_object(victim).try_resolve();
    assert_sorcery_timing_rejection(&result);
    drop(result);
    assert_eq!(runner.state().objects[&reckoning].zone, Zone::Hand);
    assert!(runner.state().stack.is_empty());
}

// ---------------------------------------------------------------------------
// Dragon Grip
// ---------------------------------------------------------------------------

/// Dragon Grip ({2}{R}) in P0's hand with three Mountains and a P0 host of
/// power `host_power`; `opponent_creature_power` adds a P1 creature of that
/// size. P1 is the active player in its main phase and P0 holds priority with
/// an empty stack, so only a flash permission lets P0 cast it.
fn dragon_grip_board(
    host_power: i32,
    opponent_creature_power: Option<i32>,
) -> (GameRunner, ObjectId, ObjectId, Vec<ObjectId>) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let host = scenario
        .add_creature(P0, "Ferocious Host", host_power, 4)
        .id();
    if let Some(power) = opponent_creature_power {
        scenario.add_creature(P1, "Opponent Brute", power, power);
    }
    let mountains = (0..3)
        .map(|_| scenario.add_basic_land(P0, ManaColor::Red))
        .collect();
    let grip = scenario
        .add_spell_to_hand(P0, "Dragon Grip", false)
        .as_enchantment()
        .with_subtypes(vec!["Aura"])
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Red],
            generic: 2,
        })
        .from_oracle_text_with_keywords(&["Enchant"], DRAGON_GRIP)
        .id();
    let mut runner = scenario.build();
    {
        let state = runner.state_mut();
        state.active_player = P1;
        state.priority_player = P0;
        state.waiting_for = WaitingFor::Priority { player: P0 };
    }
    assert!(runner.state().stack.is_empty());
    (runner, grip, host, mountains)
}

/// PRIMARY: P0 controls a 4-power creature, so ferocious lets P0 cast Dragon
/// Grip during P1's turn (CR 601.3d + CR 207.2c). Reverting the label read
/// leaves no casting option, and CR 303.1 keeps the Aura at sorcery timing.
#[test]
fn dragon_grip_ferocious_grants_flash() {
    let (mut runner, grip, host, mountains) = dragon_grip_board(4, None);
    // CR 303.4a: the Aura spell targets the creature its enchant ability names.
    let outcome = runner.cast(grip).target_object(host).resolve();
    // CR 117.3b: the active player receives priority after the spell resolves.
    assert_eq!(
        outcome.final_waiting_for(),
        &WaitingFor::Priority { player: P1 }
    );
    let state = runner.state();
    assert!(state.stack.is_empty());
    // CR 608.3c: the resolving Aura enters attached to its target.
    assert_eq!(state.objects[&grip].zone, Zone::Battlefield);
    assert_eq!(
        state.objects[&grip].attached_to,
        Some(AttachTarget::Object(host))
    );
    assert_eq!(
        mountains
            .iter()
            .filter(|mountain| state.objects[*mountain].tapped)
            .count(),
        3
    );
}

/// Without a 4-power creature, ferocious is not met and Dragon Grip keeps
/// enchantment timing (CR 117.1a + CR 303.1). Reach-guard: the positive row,
/// which differs only in the host's power.
#[test]
fn dragon_grip_without_four_power_cannot_cast_at_instant_speed() {
    let (mut runner, grip, host, _mountains) = dragon_grip_board(3, None);
    let result = runner.cast(grip).target_object(host).try_resolve();
    assert_sorcery_timing_rejection(&result);
    drop(result);
    assert_eq!(runner.state().objects[&grip].zone, Zone::Hand);
    assert!(runner.state().stack.is_empty());
}

/// Multi-authority: CR 601.3d grants flash only if its condition is met, and on
/// a spell being cast "you" is its would-be controller (CR 109.5), so P1's 5/5
/// does not satisfy P0's "you control". Reach-guard: the positive row.
#[test]
fn dragon_grip_opponents_four_power_creature_does_not_grant_flash() {
    let (mut runner, grip, host, _mountains) = dragon_grip_board(3, Some(5));
    let result = runner.cast(grip).target_object(host).try_resolve();
    assert_sorcery_timing_rejection(&result);
    drop(result);
    assert_eq!(runner.state().objects[&grip].zone, Zone::Hand);
    assert!(runner.state().stack.is_empty());
}
