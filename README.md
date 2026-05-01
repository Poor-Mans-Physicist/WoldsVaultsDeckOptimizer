# WoldsVaultsDeckOptimizer
Script that computes the best deck layouts for Wold's Vaults based on a simulated-annealing optimizer. The main simulation logic is written in Rust, and data handling is done in Python. The output is a .xlsx file containing a full breakdown of the decks and their optimal solutions. The main Python code in NDM_Optimizer_Rust.py contains instructions on how to configure the parameters in the script for testing and performance. 

In general, this optimizer uses the "NDM", a score that represents the total multiplier a single card type would recieve if every card bearing slot (non greed) were filled with it. For example, a deck with NDM = 100 filled with +1% HP cards would provide the player with +100% HP when equiped. Make sure to check the heatmaps provided with every layout to ensure that the multipliers have the distribution you want; with the way greed works the multiplier often concentrates in just a few cards.

This optimizer checks two classes of decks: "Shiny" stat card based decks, and "Evo" evolution card based decks. Both Shiny and Evolution cards have the same behavior where they scale based off the cards around them (row, column, or surrounding), but Shiny cards can have stat cores applied to them (but have lower base stats on average), and Evo cards cannot have stat cores applied (but have higher base stats to compensate). The NDMs do not reflect this base stat difference, so an Evo deck with NDM = 5000 might be stronger than a Shiny with NDM = 6000 depending on the specific ratio of stat:evo card bases for that card type. 

In general, the optimizer takes into account vanilla cores, deluxe cards and the fancy core, and greed cards when computing the optimal layout. Other Wold's cores are not supported. By default, everything is assumed to be ideal and maxed out (all greed cards are 5x, greater cores, etc.). It also has support for a few experimental cores such as the "balance core", which aims to lower overall NDM while improving the even-ness of the spread. 

This optimizer can be run for Vanilla by enabling multiplicative core scaling, turning off "positional shiny" (this converts the "Shiny" card class into a pure stat deck, which is relevent to Vanilla but not really to Wold's) and turning off the deluxe card system.

You will need to install Rust and compile the simulation core before the script can be run. This process looks like:

1. Go to https://rustup.rs and follow the instructions for your OS. Accept all defaults.
2. Install maturin and openpyxl: `pip install maturin openpyxl`
3. Compile the code (might take a bit, but drastically reduces simulation runtime. You don't need to recompile when altering parameters, only if you actually change the optimizer logic in lib.rs):

**Windows (PowerShell):**
```
cd ndm_core
python -m maturin build --release
pip install .\target\wheels\ndm_core-0.1.0-cp311-cp311-win_amd64.whl --force-reinstall
```

**Mac/Linux:**
```
cd ndm_core
python -m maturin build --release
pip install target/wheels/ndm_core-0.1.0-*.whl --force-reinstall
```

The exact filename in the `wheels/` folder may differ slightly depending on your Python version and OS — use whatever `.whl` file appears there.

4. Run the script: `python NDM_Optimizer_Rust.py`

If the Rust extension fails to import, the script will fall back to pure Python automatically and print a warning. Everything will still work, just slower.
