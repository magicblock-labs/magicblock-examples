# 🎲 Roll Dice

Simple dice rolling application using Ephemeral Rollups to demonstrate using MagicBlock's verifiable random function (VRF) to generate random numbers.

## VRF Flow

The dice account is delegated to the Ephemeral Rollup. On the ER, the program requests randomness from the VRF oracle with a client seed; the oracle fulfills the request by invoking the program's callback instruction (e.g. `CallbackRollDiceSimple`) with verified random bytes, which become the dice roll.

# Demo

<img width="508" alt="Screenshot 2025-03-27 at 18 48 50" src="https://github.com/user-attachments/assets/8b67fd33-c9b4-48f1-9a1a-92a9e8d74111" />

🌐 **Live demo:** https://roll-dice.magicblock.app

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
network. The suite rolls on the base layer and on the Ephemeral Rollup. MagicSVM's VRF
program fulfills each request one slot later through the example's callback,
and the delegated roll's result commits back to the base layer.

`yarn test` runs `cargo +stable test`: MagicSVM needs a newer Rust than the 1.89 toolchain used for program builds.

## 🚀 Launch the Frontend

To start the frontend application locally:

```bash
cd app
```

Install dependencies:

```bash
yarn install
```

Start the development server:

```bash
yarn dev
```

The application will be available at `http://localhost:3000` (or another port if 3000 is already in use).
The delegated dice demo will be available at `http://localhost:3000/delegated`.
