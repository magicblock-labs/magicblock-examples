# 🎲 Pinocchio Roll Dice

Pinocchio (no-Anchor) dice rolling program using Ephemeral Rollups and MagicBlock's verifiable random function (VRF) to generate random numbers.

For the Anchor-based variant and frontend demo, see [Roll Dice](../anchor/README.md).

## VRF Flow

The dice account is delegated to the Ephemeral Rollup. On the ER, the program requests randomness from the VRF oracle with a client seed; the oracle fulfills the request by invoking the program's callback instruction with verified random bytes, which become the dice roll.

## Software Packages

| Software   | Version | Installation Guide                                      |
| ---------- | ------- | ------------------------------------------------------- |
| **Solana** | 3.1.9   | [Install Solana](https://docs.anza.xyz/cli/install)     |
| **Rust**   | 1.89.0  | [Install Rust](https://www.rust-lang.org/tools/install) |
| **Node**   | 24.10.0 | [Install Node](https://nodejs.org/en/download/current)  |

```sh
agave-install init 3.1.9
rustup install 1.89.0
```

## Build and Test

Install dependencies and build the program:

```bash
yarn
yarn build
```

Run the Rust test suite in `tests-magicsvm-rs/` in-process with [MagicSVM](https://github.com/magicblock-labs/magicsvm):

```bash
yarn test
```

MagicSVM simulates the base layer and the Ephemeral Rollup in one process: no validators, no
network. The suite rolls on the base layer (paying the high-priority fee) and on the
Ephemeral Rollup. MagicSVM's VRF program fulfills each request one slot later
through the example's callback, and the delegated roll's result commits back
to the base layer.

`yarn test` runs `cargo +stable test`: MagicSVM needs a newer Rust than the 1.89 toolchain used for program builds.
