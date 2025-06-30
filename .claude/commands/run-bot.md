---
description: Run the bot for testing, with 5 min timeout
allowed-tools: Bash(cargo:*), Read(.env), Read(.env.staging)
---

# Run bot for testing

## Description

Environment variables:
!`cat .env.staging`

Run the bot with `TELOXIDE_TOKEN=$TELOXIDE_TOKEN RUST_LOG=$RUST_LOG cargo run`.
Put a timeout of 5 minutes on the command because its a server thats not going to exit.
