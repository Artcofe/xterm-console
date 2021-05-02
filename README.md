# bubo

Triangular arbitrage bot for crypto.com written using `tokio`.

# How to run

`cargo run --release`

# General description

1. An implementation of triangular arbitrage on crypto.com exchange.
2. The logic is pretty straightforward and is more or less easy to quickly change to test your hypothesis.
3. Bot is resource-efficient and fast enough to adapt to market changes (tested on a machine with 1 CPU and 500 MB RAM, consumes ~2% of CPU and tens of MBs of RAM at most).
4. Doesn't generate profit on the current state of the market and mostly loses money slowly, but can provide insights and technical solutions to the problems that one has to solve anyway to make this kind of software.
5. May have some non-critical bugs.
6. May have minor dumb solutions which I was too lazy to fix when I got bored to work on the bot.

# My experience

I've researched a decent amount of materials on the triangular arbitrage. You might know or hav