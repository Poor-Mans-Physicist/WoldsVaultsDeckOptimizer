# WoldsVaultsDeckOptimizer
Script that computes the best deck layouts for Wold's Vaults based on a simulated-annealing optimizer. The main simulation logic is written in rust, and data handling is done in python. The output is a .xlxs file containing a full breakdown of the decks and their optimal solutions. The mainpython code in NDM_Optimizer_Rust.py contains instructions on how to configure the parameters in the script for testing and performance. 
You will need to install rust and compile the simulation core before the script can be run. This process looks like:
1. Go to https://rustup.rs and follow the instructions for your OS. Accept all defaults.
2. Install maturin: pip install maturin
3. Compile the code (might take a bit, but drastically reduces simulation runtime. You don't need to recompile when altering parameters, only if you actually change the optimizer logic in lib.rs): 
- cd ndm_core
- python -m maturin build --release
- pip install .\target\wheels\ndm_core-0.1.0-cp311-cp311-win_amd64.whl --force-reinstall
