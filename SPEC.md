# Pool Scanner - Technical Specification

## Project Overview

**Pool Scanner** is a Rust-based blockchain analytics tool that discovers and validates decentralized exchange (DEX) liquidity pools across multiple EVM-compatible networks. It queries factory contracts to identify trading pairs and filters them by liquidity thresholds.

---

## Architecture

### Core Components

1. **Configuration Loader** - Parses TOML config files for network-specific settings
2. **Pool Discovery Engine** - Queries factory contracts for pool addresses
3. **Liquidity Validator** - Checks pool balances against minimum thresholds
4. **Output Generator** - Produces structured TOML output files

### Design Patterns

- **Async/Await** - Concurrent RPC calls using Tokio
- **Batch Processing** - Uses `CallBatchLayer` for efficient RPC requests
- **Factory Pattern** - Supports multiple DEX versions (V1/V2/V3/V4)
- **Strategy Pattern** - Different query strategies per factory type

---

## Data Structures

### ScannerConfig

```rust
pub struct ScannerConfig {
    pub rpc_url: String,                    // RPC endpoint URL
    pub multicall3_address: Address,        // Multicall3 contract
    pub stables: Vec<Address>,              // Stablecoin addresses
    pub other_tokens: Vec<Address>,         // Non-stable tokens (WETH, WBNB, etc.)
    pub factories: Vec<FactoryConfig>,      // DEX factory contracts
}
```

### FactoryConfig

```rust
pub struct FactoryConfig {
    pub name: String,                       // Human-readable name
    pub address: Address,                   // Factory contract address
    pub factory_type: PoolTypeConfig,       // V1, V2, V3, or V4
}
```

### PoolTypeConfig (Enum)

```rust
pub enum PoolTypeConfig {
    V1,  // Uniswap V1 style: getExchange(address) -> address
    V2,  // Uniswap V2 style: getPair(address, address) -> address
    V3,  // Uniswap V3 style: getPool(address, address, uint24) -> address
    V4,  // Reserved for future implementation
}
```

### PoolConfig (Output)

```rust
pub struct PoolConfig {
    pub pair: Address,                      // Pool contract address
    pub dex: String,                        // DEX name
    pub pool_type: PoolTypeConfig,          // Pool version
    pub fee_numerator: Option<u64>,         // Optional fee numerator
    pub fee_denominator: Option<u64>,       // Optional fee denominator
    pub fee: Option<u32>,                   // V3 fee tier (100, 500, 3000, 10000)
}
```

---

## Business Logic

### Pair Generation Strategy

The scanner generates token pairs using two strategies:

1. **Stable vs Other**: Each stablecoin paired with each non-stable token
2. **Stable vs Stable**: Each unique pair of stablecoins (no duplicates)

```rust
// Example: 2 stables × 1 other = 2 pairs
// Example: 2 stables × 2 stables (unique) = 1 pair
```

### Pool Query Strategy by Type

#### V1 Factories
- Query: `getExchange(token)` for each non-stable token
- No pair combinations needed

#### V2 Factories
- Query: `getPair(tokenA, tokenB)` for all generated pairs
- Single query per pair

#### V3 Factories
- Query: `getPool(tokenA, tokenB, fee)` for all pairs × all fee tiers
- Fee tiers: 100, 500, 3000, 10000 (0.01%, 0.05%, 0.3%, 1%)

#### V4 Factories
- Not implemented (placeholder)

### Liquidity Filtering

Pools are validated by checking stablecoin balances:

| Network | Minimum Balance | Token Decimals |
|---------|----------------|----------------|
| Ethereum | 50,000 USDC/USDT | 6 decimals |
| BNB Chain | 50,000 USDT/USDC | 18 decimals |

Threshold values:
- **ETH**: `50_000_000_000` (50,000 × 10^6)
- **BNB**: `50_000_000_000_000_000_000_000` (50,000 × 10^18)

### Deduplication

1. Sort pools by address
2. Remove consecutive duplicates by address
3. Ensures each pool appears only once in output

---

## Configuration Schema

### Network Config File (TOML)

```toml
# RPC endpoint for the network
rpc_url = "https://..."

# Multicall3 contract address (for batch calls)
multicall3_address = "0x..."

# Stablecoin addresses (used for liquidity checks)
stables = [
    "0x...", # USDC
    "0x...", # USDT
]

# Non-stable tokens to pair with stables
other_tokens = [
    "0x...", # WETH/WBNB
]

# DEX factory contracts
[[factories]]
name = "UniswapV2"
address = "0x..."
type = "v2"

[[factories]]
name = "UniswapV3"
address = "0x..."
type = "v3"
```

### Supported Factory Types

| Type | Interface Method | Fee Support |
|------|-----------------|-------------|
| `v1` | `getExchange(address)` | No |
| `v2` | `getPair(address, address)` | No |
| `v3` | `getPool(address, address, uint24)` | Yes (4 tiers) |
| `v4` | Not implemented | TBD |

---

## Execution Flow

```
1. Load Configuration
   └─> Read NETWORK env var (default: "eth")
   └─> Load config/{network}.toml
   └─> Parse into ScannerConfig

2. Setup Provider
   └─> Parse RPC URL
   └─> Create HTTP provider with CallBatchLayer
   └─> Configure batch timeout (10ms)

3. Generate Pair Combinations
   └─> Stable × Other tokens
   └─> Stable × Stable (unique pairs)

4. Build Pool Discovery Calls
   └─> For each factory:
       ├─> V1: Query getExchange for each token
       ├─> V2: Query getPair for each pair
       ├─> V3: Query getPool for each pair × fee tier
       └─> V4: Skip (placeholder)

5. Execute Batch RPC Calls
   └─> Join all pool discovery futures
   └─> Filter out zero addresses

6. Validate Liquidity
   └─> For each pool × stablecoin:
       ├─> Query balanceOf(pool)
       └─> Compare against threshold

7. Filter & Deduplicate
   └─> Keep pools above threshold
   └─> Sort by address
   └─> Remove duplicates

8. Generate Output
   └─> Serialize to TOML
   └─> Write to {network}_output.toml
```

---

## Error Handling

### Configuration Errors
- Missing config file → Panic with path message
- Invalid TOML syntax → Propagate `toml::de::Error`
- Invalid address format → Propagate from `alloy::primitives`

### RPC Errors
- Connection failures → Propagate `eyre::Error`
- Contract call failures → Log and return `Address::ZERO`
- Invalid URL → Propagate `url::ParseError`

### Runtime Errors
- File I/O errors → Propagate `std::io::Error`
- Serialization errors → Propagate `toml::ser::Error`

---

## Dependencies

### Core Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `alloy` | 1.7 | Ethereum RPC & contract interaction |
| `tokio` | 1 | Async runtime |
| `serde` | 1.0 | Serialization/deserialization |
| `toml` | 0.8 | TOML parsing & generation |
| `futures` | 0.3 | Async utilities |
| `eyre` | 0.6 | Error handling |
| `url` | 2 | URL parsing |

### Alloy Features

```toml
alloy = { 
    version = "1.7", 
    features = [
        "provider-http",    # HTTP provider
        "provider-ws",      # WebSocket provider
        "signers",          # Signer traits
        "signer-local",     # Local keystore signer
        "sol-types",        # Solidity type bindings
        "contract",         # Contract bindings
        "rpc-types",        # RPC types
        "network",          # Network types
        "consensus",        # Consensus types
    ] 
}
```

---

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `NETWORK` | `eth` | Network config to use (`eth` or `bnb`) |

---

## File Structure

```
pool_scanner/
├── config/
│   ├── eth.toml              # Ethereum mainnet config
│   └── bnb.toml              # BNB Smart Chain config
├── src/
│   └── main.rs               # Main application logic
├── tests/                    # Test suite (to be added)
│   ├── unit/
│   │   ├── config_tests.rs   # Configuration parsing tests
│   │   └── filter_tests.rs   # Liquidity filter tests
│   └── integration/
│       └── scanner_tests.rs  # End-to-end tests
├── .github/
│   └── workflows/
│       └── ci.yml            # CI/CD pipeline
├── Cargo.toml                # Project manifest
├── SPEC.md                   # This specification
├── README.md                 # User documentation
├── eth_output.toml           # Ethereum scan results (generated)
└── bnb_output.toml           # BNB scan results (generated)
```

---

## Testing Strategy

### Unit Tests

1. **Configuration Parsing**
   - Valid config deserialization
   - Invalid config error handling
   - Factory type variants

2. **Pair Generation**
   - Stable × Other combinations
   - Stable × Stable unique pairs
   - Edge cases (empty lists)

3. **Liquidity Filtering**
   - Above threshold → included
   - Below threshold → excluded
   - Edge case: exactly at threshold

4. **Deduplication**
   - Duplicate removal
   - Order preservation

### Integration Tests

1. **End-to-End Scan**
   - Test with mock RPC
   - Verify output format

2. **Multi-Network Support**
   - ETH config loading
   - BNB config loading

### Mocking Strategy

- Use `wiremock` for HTTP RPC mocking
- Mock factory contract responses
- Mock ERC20 balance responses

---

## CI/CD Pipeline

### GitHub Actions Workflow

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --all-features

  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy -- -D warnings

  format:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cargo fmt --check

  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo build --release
```

---

## Security Considerations

### Private Key Handling
- **No private keys** required (read-only operations)
- RPC URLs are public endpoints

### Configuration Security
- Avoid committing private RPC endpoints
- Use environment variables for sensitive data in production

### Input Validation
- Address format validation (via `alloy::primitives::Address`)
- URL validation (via `url::Url`)
- TOML schema validation (via `serde`)

---

## Performance Optimizations

### Batch RPC Calls
- Uses `CallBatchLayer` with 10ms wait time
- Groups multiple contract calls into single RPC request
- Reduces network overhead

### Parallel Execution
- All pool discovery calls run concurrently
- All balance checks run concurrently
- Uses `futures::future::join_all`

### Memory Efficiency
- Streams results instead of loading all into memory
- Deduplicates after filtering (smaller dataset)

---

## Extensibility

### Adding New Networks

1. Create `config/{network}.toml`
2. Define RPC URL, tokens, and factories
3. Set `NETWORK={network}` environment variable

### Adding New DEX Factories

1. Add factory entry to config TOML
2. Ensure factory follows V1/V2/V3 interface
3. No code changes required

### Adding V4 Support

1. Define V4 factory interface
2. Implement pool discovery logic
3. Add fee structure if applicable

---

## Known Limitations

1. **V4 Not Supported** - Placeholder only
2. **Fixed Fee Tiers** - V3 uses hardcoded fee tiers (100, 500, 3000, 10000)
3. **Single RPC URL** - No fallback or load balancing
4. **No Pagination** - Assumes all pairs fit in memory
5. **Hardcoded Thresholds** - Network-specific liquidity thresholds in code

---

## Future Enhancements

1. **Dynamic Fee Discovery** - Query factory for supported fee tiers
2. **Multi-RPC Support** - Load balancing across multiple endpoints
3. **Streaming Output** - Write pools as discovered (memory efficiency)
4. **GraphQL Support** - Use The Graph for supported networks
5. **Pool Metadata** - Include reserve amounts, APR, volume
6. **Arbitrage Detection** - Identify price discrepancies across pools
7. **WebSocket Subscriptions** - Real-time pool updates

---

## Usage Examples

### Basic Usage

```bash
# Scan Ethereum
NETWORK=eth cargo run

# Scan BNB Chain
NETWORK=bnb cargo run

# Custom network config
NETWORK=arbitrum cargo run
```

### Programmatic Usage

```rust
use pool_scanner::{ScannerConfig, PoolTypeConfig};

// Load config
let config = toml::from_str::<ScannerConfig>(&config_str)?;

// Access tokens
for stable in &config.stables {
    println!("Stablecoin: {:?}", stable);
}

// Access factories
for factory in &config.factories {
    match factory.factory_type {
        PoolTypeConfig::V2 => {
            println!("V2 Factory: {} at {:?}", factory.name, factory.address);
        }
        PoolTypeConfig::V3 => {
            println!("V3 Factory: {} at {:?}", factory.name, factory.address);
        }
        _ => {}
    }
}
```

---

## Troubleshooting

### Common Issues

**1. Config File Not Found**
```
Failed to read config file at config/eth.toml
```
- Ensure running from project root
- Check file exists at expected path

**2. RPC Connection Failed**
```
transport error
```
- Verify RPC URL is accessible
- Check network connectivity
- Try alternative RPC endpoint

**3. No Pools Found**
```
Found 0 pools
```
- Verify token addresses are correct
- Check liquidity threshold (may be too high)
- Confirm factory addresses are valid

**4. Out of Memory**
- Reduce number of token pairs in config
- Increase system memory

---

## License

MIT

---

## Contributing

1. Fork the repository
2. Create feature branch (`git checkout -b feature/amazing-feature`)
3. Commit changes (`git commit -m 'Add amazing feature'`)
4. Push to branch (`git push origin feature/amazing-feature`)
5. Open Pull Request

### Code Style

- Follow Rust idioms and conventions
- Run `cargo fmt` before committing
- Run `cargo clippy` for linting
- Add tests for new functionality
- Update documentation

---

## Contact

Project: Arbitraj Pool Scanner  
Repository: `/Users/boncho/projects/personal/arbitraj/pool_scanner`
