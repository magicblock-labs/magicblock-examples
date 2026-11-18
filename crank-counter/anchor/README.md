# ⚙️ Crank Counter

Simple counter program using Anchor and Ephemeral Rollups with scheduled cranks for automatic execution.

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
network. The suite delegates the counter, schedules an increment crank on the
Ephemeral Rollup, warps through its deadlines, and checks that every run
increments the counter and that the result commits back to the base layer.

`yarn test` runs `cargo +stable test`: MagicSVM needs a newer Rust than the 1.89 toolchain used for program builds.

## ⏱️ Scheduling a Crank

This example schedules an instruction to run automatically on the Ephemeral Rollup. After delegating the counter PDA, the program's `schedule_increment` instruction CPIs into the MagicBlock magic program with a `ScheduleTask`, which repeatedly invokes `increment` on a fixed interval for a number of iterations — no client transaction needed per tick.

```rust
pub fn schedule_increment(
    ctx: Context<ScheduleIncrement>,
    args: ScheduleIncrementArgs, // task_id, execution_interval_millis, iterations
) -> Result<()> {
    let increment_ix = Instruction {
        program_id: crate::ID,
        accounts: vec![AccountMeta::new(ctx.accounts.counter.key(), false)],
        data: anchor_lang::InstructionData::data(&crate::instruction::Increment {}),
    };

    let ix_data = bincode::serialize(&MagicBlockInstruction::ScheduleTask(ScheduleTaskArgs {
        task_id: args.task_id,
        execution_interval_millis: args.execution_interval_millis,
        iterations: args.iterations,
        instructions: vec![increment_ix],
    }))?;
    // ... CPI into MAGIC_PROGRAM_ID with the serialized ScheduleTask
}
```
