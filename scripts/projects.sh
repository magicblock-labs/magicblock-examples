#!/bin/bash
# Single source of truth for the example projects. The project name is the stable
# CLI/CI identifier; project_dir maps that identifier to its location in the
# use-case/framework folder layout. Every project exposes `yarn build` and `yarn test`,
# which runs its Rust MagicSVM suite under tests-magicsvm-rs/ in-process.
#
# Sourced by:
#   - scripts/test-magicsvm.sh (local runner)
#   - the CI matrix generator (magicsvm_projects_json)
#
# Keep this list in sync when adding/removing examples; both the local runner and
# CI pick it up automatically.

MAGICSVM_PROJECTS=(anchor-counter rust-counter pinocchio-counter private-counter pinocchio-private-counter oracle-priced-purchase oncurve-delegation rock-paper-scissor ephemeral-account-chats session-keys binary-prediction magic-actions delegation-actions spl-tokens crank-counter roll-dice pinocchio-roll-dice rewards-delegated-vrf gasless-starter gachapon-example sealed-auction)

# Print the directory for a stable project name, or empty if unknown.
project_dir() {
  case "$1" in
    anchor-counter) echo "counter/anchor" ;;
    pinocchio-counter) echo "counter/pinocchio" ;;
    rust-counter) echo "counter/native-rust" ;;
    binary-prediction) echo "binary-prediction/anchor" ;;
    oracle-priced-purchase) echo "oracle-priced-purchase/anchor" ;;
    crank-counter) echo "crank-counter/anchor" ;;
    delegation-actions) echo "delegation-actions/anchor" ;;
    ephemeral-account-chats) echo "ephemeral-account-chats/anchor" ;;
    gachapon-example) echo "gachapon-example" ;;
    gasless-starter) echo "gasless-starter/anchor" ;;
    magic-actions) echo "magic-actions/anchor" ;;
    session-keys) echo "session-keys/anchor" ;;
    spl-tokens) echo "spl-tokens/anchor" ;;
    rewards-delegated-vrf) echo "rewards-delegated-vrf/anchor" ;;
    roll-dice) echo "roll-dice/anchor" ;;
    pinocchio-roll-dice) echo "roll-dice/pinocchio" ;;
    private-counter) echo "private-counter/anchor" ;;
    pinocchio-private-counter) echo "private-counter/pinocchio" ;;
    rock-paper-scissor) echo "rock-paper-scissor/anchor" ;;
    sealed-auction) echo "sealed-auction/anchor" ;;
    oncurve-delegation) echo "oncurve-delegation/client" ;;
    *) echo "" ;;
  esac
}

magicsvm_projects_json() {
  local out="" p
  for p in "${MAGICSVM_PROJECTS[@]}"; do
    out+="\"$p\","
  done
  echo "[${out%,}]"
}
