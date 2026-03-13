# Pool Scanner

A Rust-based tool for scanning and discovering decentralized exchange (DEX) liquidity pools across multiple blockchain networks.

## Overview

Pool Scanner queries factory contracts from various DEXes (Uniswap V1/V2/V3, PancakeSwap, Biswap, etc.) to discover trading pools between specified token pairs. It filters pools by minimum liquidity thresholds and outputs the results in TOML format for further processing.

## Features

- **Multi-chain support**: Ethereum and BNB Smart Chain configurations included
- **Multiple DEX versions**: Supports Uniswap V1, V2, V3 factory contracts
- **Batch RPC calls**: Uses `CallBatchLayer` for efficient parallel RPC requests
- **Liquidity filtering**: Filters pools by minimum stablecoin balance
- **Configurable**: TOML-based configuration for easy customization
- **Deduplication**: Automatically removes duplicate pool addresses
- **Comprehensive tests**: 32 unit and integration tests
- **CI/CD**: GitHub Actions workflow for automated testing and linting

## Project Structure

```
pool_scanner/
├── config/          # Network configuration files
│   ├── eth.toml     # Ethereum mainnet config
│   └── bnb.toml     # BNB Smart Chain config
├── src/
│   ├── main.rs      # Main scanner implementation
│   └── lib.rs       # Library with testable functions
├── tests/           # Integration tests
│   └── integration_tests.rs
├── .github/
│   └── workflows/
│       └── ci.yml   # GitHub Actions CI workflow
├── Cargo.toml       # Rust dependencies
├── SPEC.md          # Technical specification
├── README.md        # This file
├── eth_output.toml  # Ethereum pools (generated)
└── bnb_output.toml  # BNB pools (generated)
```

## Configuration

Configuration files are located in the `config/` directory. Each network has its own TOML file with the following structure. `min_liquidity` is specified in normalized units and converted to on-chain amounts using `decimals` (defaults to `18` if omitted):

```toml
rpc_url = "https://..."                    # RPC endpoint
multicall3_address = "0x..."               # Multicall3 contract address

[[tokens]]
address = "0x..."
symbol = "USDC"
decimals = 6
min_liquidity = "50000"                    # normalized amount

[[tokens]]
address = "0x..."
symbol = "WETH"
decimals = 18

[[factories]]                              # DEX factory contracts
name = "UniswapV2"
address = "0x..."
type = "v2"
```

### Factory Types

Supported factory types:
- `v1` - Uniswap V1 style (getExchange)
- `v2` - Uniswap V2 style (getPair)
- `v3` - Uniswap V3 style (getPool with fee tiers: 100, 500, 3000, 10000)
- `v4` - Reserved for future implementation

## Usage

### Environment Variables

- `NETWORK` - Network configuration to use (default: `eth`)
  - Set to `eth` for Ethereum mainnet
  - Set to `bnb` for BNB Smart Chain

### Running the Scanner

```bash
# Scan Ethereum mainnet
NETWORK=eth cargo run

# Scan BNB Smart Chain
NETWORK=bnb cargo run

# Build and run in release mode
NETWORK=eth cargo run --release
```

### Output

The scanner generates output files in the project root:
- `eth_output.toml` - Ethereum pools
- `bnb_output.toml` - BNB pools

Example output:
```toml
[[pools]]
pair = "0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640"
dex = "UniswapV3"
pool_type = "v3"
token0 = { address = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", symbol = "USDC" }
token1 = { address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2", symbol = "WETH" }
fee = 500

[[pools]]
pair = "0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc"
dex = "UniswapV2"
pool_type = "v2"
token0 = { address = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", symbol = "USDC" }
token1 = { address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2", symbol = "WETH" }
```

## Testing

The project includes comprehensive test coverage with unit and integration tests.

### Running Tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run only unit tests
cargo test --lib

# Run only integration tests
cargo test --test integration_tests

# Run specific test
cargo test test_generate_pairs_stable_vs_other
```

### Test Coverage

**Unit Tests** (23 tests in `src/lib.rs`):
- Configuration parsing (6 tests)
- Pair generation (5 tests)
- Liquidity filtering (7 tests)
- Pool deduplication (5 tests)

**Integration Tests** (9 tests in `tests/integration_tests.rs`):
- Config file loading
- End-to-end processing
- Network thresholds
- Output format validation
- Large-scale processing

## Building

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Build documentation
cargo doc --open
```

## CI/CD

The project uses GitHub Actions for continuous integration. The CI pipeline runs on every push and pull request:

- **Test Suite**: Runs all unit and integration tests
- **Clippy Lint**: Checks for code quality and common mistakes
- **Rustfmt**: Verifies code formatting
- **Build Release**: Builds optimized release binary
- **Documentation**: Generates and validates Rust docs
- **Security Audit**: Checks dependencies for known vulnerabilities

### CI Workflow Jobs

```yaml
- Test Suite (ubuntu-latest)
- Clippy Lint (ubuntu-latest)
- Rustfmt (ubuntu-latest)
- Build Release (ubuntu-latest)
- Documentation (ubuntu-latest)
- Security Audit (ubuntu-latest)
```

To run CI checks locally before pushing:

```bash
# Format check
cargo fmt --check

# Lint check
cargo clippy --all-features --all-targets -- -D warnings

# Run tests
cargo test --all-features

# Build release
cargo build --release
```

## License

MIT
