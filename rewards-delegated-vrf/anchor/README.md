# ➕ Rewards with Delegated VRF

Anchor rewards program that mints random rewards on an Ephemeral Rollup using MagicBlock's verifiable random function (VRF).

🌐 **Live demo (rewards dashboard):** https://magicblock-rewards-dashboard.vercel.app

## VRF Flow

A reward account is delegated to the Ephemeral Rollup. On the ER, the program requests randomness from the VRF oracle; the oracle fulfills the request by invoking the program's callback instruction with verified random bytes, which the program uses to determine the reward. State is then committed back to the base layer.

## Software Packages

This program has utilized the following software packages.

| Software   | Version | Installation Guide                                              |
| ---------- | ------- | --------------------------------------------------------------- |
| **Solana** | 3.1.9   | [Install Solana](https://docs.anza.xyz/cli/install)             |
| **Rust**   | 1.89.0  | [Install Rust](https://www.rust-lang.org/tools/install)         |
| **Anchor** | 1.0.2   | [Install Anchor](https://www.anchor-lang.com/docs/installation) |
| **Node**   | 24.10.0 | [Install Node](https://nodejs.org/en/download/current)          |

```sh
agave-install init 3.1.9
rustup install 1.89.0
avm use 1.0.2
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
network. The suite draws rewards through VRF callbacks on the Ephemeral Rollup and on
the base layer, pays SPL rewards until they run out, checks that only admins
and whitelisted signers can request draws, and covers admin and whitelist
transfers and reward updates.

`yarn test` runs `cargo +stable test`: MagicSVM needs a newer Rust than the 1.89 toolchain used for program builds.

## Notes

You may need to update the example program id and authority:

```rust
declare_id!("HuGRGfqr7BNdeogipmidXL21PjF4qSoXFDaCBhetviwZ");
pub const PROGRAM_AUTHORITY: Pubkey = pubkey!("EyBRt4Acr7b4s3exfnVvJ4EgL8oa6Lc4JK1Leonud34W");
```
