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

I've researched a decent amount of materials on the triangular arbitrage. You might know or have a feeling that there is not a lot of quality content on this topic - and you are right. This is the rare case of the business field where the only way to test your ideas is to actually implement them.
There are still some research papers and articles on the topic. Unfortunately, most of them are out of touch with the realities of trading. This is a common illness for a lot of research papers, and arbitrage is not an exception - the most of those materials are about solving a task of finding arbitrage opportunities by representing an exchange as a graph, solving a problem of detecting the negative cycles on it, etc., etc. But in practice it turns out that writing a brute-force solution and trying to optimize it is better for a few reasons:
1. Finding arbitrage oppotunities is not the main problem if you want to write a program that earns money. Triangular arbitrage for an individual chain  is a few multiplications and divisions which can be 