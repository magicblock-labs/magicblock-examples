# 🌐 Live Demos

Every example with a deployed frontend, recorded against Solana devnet and MagicBlock Ephemeral
Rollups. Each GIF shows real transactions; click a heading to open the live app.

| Example | Live demo | Source |
| ------- | --------- | ------ |
| [Counter](#counter) | [counter-example.magicblock.app](https://counter-example.magicblock.app) | [counter/anchor](./counter/anchor/) |
| [Private Counter](#private-counter) | [private-counter-example.magicblock.app](https://private-counter-example.magicblock.app) | [private-counter/anchor](./private-counter/anchor/) |
| [Session Keys](#session-keys) | [counter-session-keys-example.magicblock.app](https://counter-session-keys-example.magicblock.app) | [session-keys/anchor](./session-keys/anchor/) |
| [Roll Dice](#roll-dice) | [roll-dice.magicblock.app](https://roll-dice.magicblock.app) | [roll-dice/anchor](./roll-dice/anchor/) |
| [Binary Prediction](#binary-prediction) | [binary-prediction.magicblock.app](https://binary-prediction.magicblock.app) | [binary-prediction/anchor](./binary-prediction/anchor/) |
| [SPL Tokens](#spl-tokens) | [spl-demo.magicblock.app](https://spl-demo.magicblock.app) | [spl-tokens/anchor](./spl-tokens/anchor/) |
| [Rock Paper Scissor](#rock-paper-scissor) | [rps-example.magicblock.app](https://rps-example.magicblock.app) | [rock-paper-scissor/anchor](./rock-paper-scissor/anchor/) |
| [Rewards (Delegated VRF)](#rewards-delegated-vrf) | [magicblock-rewards-dashboard.vercel.app](https://magicblock-rewards-dashboard.vercel.app) | [rewards-delegated-vrf/anchor](./rewards-delegated-vrf/anchor/) |
| [Gasless Starter](#gasless-starter) | [gasless-starter.vercel.app](https://gasless-starter.vercel.app) | [gasless-starter/anchor](./gasless-starter/anchor/) |

## [Counter](https://counter-example.magicblock.app)

Increment on Solana, delegate the counter to an Ephemeral Rollup, increment there in ~150–300 ms
per transaction, then undelegate to commit the final count back to Solana.

![Counter demo](https://github.com/user-attachments/assets/208b15dd-370d-4e65-b4d7-4aaa5dbe2444)

<details><summary>Screenshot</summary>

![Counter screenshot](https://github.com/user-attachments/assets/c4e58e51-1128-4e47-93bb-52b4bbd453a9)

</details>

## [Private Counter](https://private-counter-example.magicblock.app)

The same counter on a TEE-backed Private Ephemeral Rollup. Flip the toggle to make the delegated
count private: only permitted readers see the value, everyone else sees it blurred.

![Private Counter demo](https://github.com/user-attachments/assets/42773126-bda4-46d4-ae6d-66c7b180ffaf)

<details><summary>Screenshot</summary>

![Private Counter screenshot](https://github.com/user-attachments/assets/605974a3-b1c1-4a38-84b7-b5b1d94668b2)

</details>

## [Session Keys](https://counter-session-keys-example.magicblock.app)

Approve one session with the wallet, then every increment on Solana and on the Ephemeral Rollup
is signed by the session key, with no further wallet prompts.

![Session Keys demo](https://github.com/user-attachments/assets/47921566-9f95-494d-bfad-4b1abbe9e461)

<details><summary>Screenshot</summary>

![Session Keys screenshot](https://github.com/user-attachments/assets/e48390f6-d40f-4543-85bf-94d972ea7dbb)

</details>

## [Roll Dice](https://roll-dice.magicblock.app)

The dice account is delegated to an Ephemeral Rollup. Each roll requests verifiable randomness
from the VRF oracle, and the result lands in under 200 ms.

![Roll Dice demo](https://github.com/user-attachments/assets/0b72e83a-96e4-439d-a8ea-65139f32592c)

<details><summary>Screenshot</summary>

![Roll Dice screenshot](https://github.com/user-attachments/assets/251d2757-0688-4bf3-b268-0cb2a23aa468)

</details>

## [Binary Prediction](https://binary-prediction.magicblock.app)

Bet on whether SOL/USD goes up or down over a few seconds. The market runs on an Ephemeral Rollup
against a live price oracle; tickets and settlement are sent in ~50 ms.

![Binary Prediction demo](https://github.com/user-attachments/assets/0a307511-11f6-430d-87b9-24a09e974f1a)

<details><summary>Screenshot</summary>

![Binary Prediction screenshot](https://github.com/user-attachments/assets/eca0553f-f1cf-4842-abcd-632972cc55b1)

</details>

## [SPL Tokens](https://spl-demo.magicblock.app)

Delegate SPL token balances into ephemeral ATAs, transfer between accounts on the Ephemeral
Rollup, then undelegate and withdraw back to Solana.

![SPL Tokens demo](https://github.com/user-attachments/assets/5c1fb57f-bd4d-46d5-b1c7-78b781a4fd74)

<details><summary>Screenshot</summary>

![SPL Tokens screenshot](https://github.com/user-attachments/assets/f9ab89f0-1599-4f17-8902-8f2b255d1ceb)

</details>

## [Rock Paper Scissor](https://rps-example.magicblock.app)

Play a best-of-three against a robot. Both moves stay private inside a TEE until the round is
revealed; the "Under the hood" log links every Solana and TEE transaction.

![Rock Paper Scissor demo](https://github.com/user-attachments/assets/cd078ada-41a0-469f-85e1-fc5207e8b05e)

<details><summary>Screenshot</summary>

![Rock Paper Scissor screenshot](https://github.com/user-attachments/assets/9d09dfe9-81b4-472f-b93d-eaee3c8d0754)

</details>

## [Rewards (Delegated VRF)](https://magicblock-rewards-dashboard.vercel.app)

Admin dashboard for a reward distributor: configure a reward list, delegate it to an Ephemeral
Rollup, and let VRF pick rewards for users. The recording connects a wallet, shows the distributor
and its schedule, tops up the callback escrow, and walks through the available actions.

![Rewards demo](https://github.com/user-attachments/assets/94cd3eed-e02a-4b80-82e6-c0aaf31fc6e8)

<details><summary>Screenshot</summary>

![Rewards screenshot](https://github.com/user-attachments/assets/4d91dab1-d7b1-4d6f-8e3b-a0bc46e4bee0)

</details>

## [Gasless Starter](https://gasless-starter.vercel.app)

A wallet with zero SOL moves USDC onto an Ephemeral Rollup through the MagicBlock Payments API,
tips in ~100 ms on the rollup for free, and withdraws back to Solana. Each move costs a 0.20 USDC
relay fee instead of SOL gas.

![Gasless Starter demo](https://github.com/user-attachments/assets/cebc76f9-b165-42a6-a6bd-bbaefc8c8afe)

<details><summary>Screenshot</summary>

![Gasless Starter screenshot](https://github.com/user-attachments/assets/f36284a0-fa70-4c20-bbf0-202745b2caad)

</details>
