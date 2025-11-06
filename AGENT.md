# agent.md — Ethereum Trading MCP Server

**
** Language: **Rust (Tokio)** · Ethereum client: **ethers-rs** · Protocol: **MCP** (rmcp SDK) · Logging: **tracing**.

---

## 0) Scope & Deliverables

**Goal:** Implement an **MCP Server** exposing three tools:

1. **`get_balance`** — Query on-chain balances for native **ETH** and any **ERC-20**.
2. **`get_token_price`** — Get token price (prefer **Chainlink** oracles; fall back to **DEX** spot price). Supports **USD** or **ETH** quotes.
3. **`swap_tokens`** — **Construct real Uniswap V2/V3 router transactions** and **simulate** via RPC (**`eth_call`** / **`eth_estimateGas`**). **Do not broadcast**.

**Hard Requirements (Technical Stack):**

* **Rust** with **Tokio** (async runtime)
* **Ethereum RPC client: ****ethers-rs** (or Alloy)
* **MCP SDK for Rust: ****rmcp** (or manual JSON-RPC 2.0)
* **Structured logging: ****tracing**
* **Connect to a ****real Ethereum RPC** (public endpoints or Alchemy/Infura)
* **Implement ****basic wallet management** for signing (prefer config file for secrets; env vars optional fallback)
* **Use ****`rust_decimal`** (or similar) for financial precision

**Deliverables:**

* **Compilable, runnable Rust project (binary MCP server)**
* **README** (setup, env vars, run, sample tool calls, design decisions, known limits)
* **Tests** (at least balance, price, and swap simulation)

---

## 1) System Architecture

**Layering:** Implement the server in three layers where each higher layer depends on the layer below it.

1. **Top MCP Layer** — rmcp server bootstrap, tool registration, JSON-RPC stdio handling.
2. **Middle Service Layer** — owns the tool-facing API surface (`get_balance`, `get_token_price`, `swap_tokens`), orchestrates shared context (providers, wallets, caches), and is covered by unit tests exercising service-level orchestration.
3. **Implementation Layer** — per-tool modules encapsulating Ethereum RPC calls, pricing logic, and swap simulation primitives, each with dedicated unit tests covering their core functionality.

```
+-----------------------+
| LLM Client / IDE      |  (Claude / Cursor / Cline)
| MCP Host              |
+-----------+-----------+
            | stdio (JSON-RPC 2.0)
            v
+-----------------------+        +----------------------------+
| MCP Server (this)     |  HTTP  | Ethereum RPC Provider      |
| - rmcp runtime        +------->+ (Alchemy/Infura/Public)    |
| - tool registry       |        | JSON-RPC: eth_*            |
| - handlers            |        +----------------------------+
|   get_balance         |  +------------------+
|   get_token_price     |  | Chainlink Feeds  |
|   swap_tokens         |  | Uniswap V2/V3    |
+-----------------------+  +------------------+
```

* **MCP Server**: invoked by the host as a **subprocess**, communicating over **stdin/stdout** (not HTTP unless you add one).
* **Ethereum RPC**: JSON-RPC over HTTPS/WSS; use `eth_call`, `eth_getBalance`, `eth_estimateGas`, `eth_sendRawTransaction` (last one optional).
* **Layer Dependencies**: The MCP layer delegates to the service layer, which in turn depends on the implementation layer modules.

---

## 2) Configuration & Secrets

**Configuration file (preferred)** — e.g., `Config.toml` or `config.yaml`, loaded at startup:

* `eth_rpc_url` — mainnet RPC (HTTPS), e.g. `https://.../v2/<KEY>`
* `private_key` — hex private key (with/without `0x`); used for signing if needed (do **not** log)
* `default_chain_id` — `1` (Ethereum mainnet)

**Environment variables (fallback)** — mirror the same keys if the config file is absent.

**Use **`dotenvy` to load fallback envs in development. Never print secrets. Provide basic wallet management that loads the signer from the config file (or environment variable fallback) for transaction signing when required.

---

## 3) Core Types (suggested)

```
pub struct BalanceOut {
  pub symbol: String, pub raw: String, pub decimals: u32, pub formatted: String
}

pub struct PriceOut {
  pub base: String, pub quote: String, pub price: String
}

pub struct SwapSimInput {
  pub from_token: Address, pub to_token: Address,
  pub amount_in_wei: U256, pub slippage_bps: u32,
  pub fee: u32, pub recipient: Address
}

pub struct SwapSimOut {
  pub amount_out_estimate: String, pub gas_estimate: String, pub calldata_hex: String
}
```

---

## 4) Tool: `get_balance`

Query ETH and ERC20 token balances

* Input: wallet address, optional token contract address
* Output: balance information with proper decimals

### Behavior

* **ETH branch** (`token == null`)
  * **Call **`eth_getBalance(address, latest)` → `raw`
  * `decimals = 18`, `symbol = "ETH"`
* **ERC-20 branch** (`token != null`)
  * **Query token contract (read-only):**
    * `balanceOf(address) -> uint256`
    * `decimals() -> uint8` (**required**)
    * `symbol() -> string` (optional; fallback to `"ERC20"` on error)

### Formatting

* `formatted = raw / 10^decimals`
* **Use ****`rust_decimal`** (no `f64`); input/output as **decimal strings**.

### MCP I/O (JSON Schemas)

**Input**

```
{"type":"object","properties":{"address":{"type":"string"},"token":{"type":["string","null"]}},"required":["address"]}
```

**Output**

```
{"type":"object","properties":{"symbol":{"type":"string"},"raw":{"type":"string"},"decimals":{"type":"integer"},"formatted":{"type":"string"}},"required":["symbol","raw","decimals","formatted"]}
```

### Example

**Request**

```
{"method":"get_balance","params":{"address":"0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"}}
```

**Response**

```
{"symbol":"ETH","raw":"1234567890000000000","decimals":18,"formatted":"1.23456789"}
```

**Pitfalls**

* **Address validation (0x + 40 hex chars)**
* **If **`decimals()` fails → error (don’t guess)
* **Rate limits → small backoff and retry once**

---

## 5) Tool: `get_token_price`

Get current token price in USD or ETH

* Input: token address or symbol
* Output: price data

### Price Sources (priority)

1. **Chainlink** (AggregatorV3Interface)
   * `latestRoundData()` → `answer` (integer) + `decimals()` (scale)
   * **Common feeds: **`ETH/USD`, `USDC/USD`, `BTC/USD`, `SOL/USD`, etc.
   * `token/ETH = (token/USD) / (ETH/USD)`
   * **Note: Oracles are ****per-chain**. On Ethereum you read **Ethereum-deployed** feeds (not cross-chain).
2. **DEX spot price (fallback)**
   * **Uniswap V2:**`price ≈ reserveY / reserveX` (careful with decimals)
   * **Uniswap V3:** read `sqrtPriceX96` or simply ask **QuoterV2** with a small `amountIn`

### MCP I/O (JSON Schemas)

**Input**

```
{"type":"object","properties":{"token":{"type":"string"},"quote":{"type":"string","enum":["USD","ETH"]}},"required":["token"]}
```

**Output**

```
{"type":"object","properties":{"base":{"type":"string"},"quote":{"type":"string"},"price":{"type":"string"}},"required":["base","quote","price"]}
```

### Behavior

* **Default **`quote = "USD"`
* **Try **`symbol → Chainlink feed` map first; if missing, fallback to DEX price (or return unsupported per your policy).
* **Optional: staleness check via **`updatedAt`; log warnings if stale.

### Known Limits (document in README)

* **Fallback DEX price can be manipulated on thin liquidity pools; treat as approximation.**
* **Only mapped Chainlink feeds are supported unless extended.**

---

## 6) Tool: `swap_tokens` (simulate real tx)

> **Build a ****real** Uniswap router transaction (V2 or V3), then **simulate** with RPC (`eth_call`/`eth_estimateGas`). **Do not broadcast**.

Execute a token swap on Uniswap V2 or V3

* Input: from_token, to_token, amount, slippage tolerance
* Output: simulation result showing estimated output and gas costs
* **Important** : Construct a real Uniswap transaction and submit it to the blockchain for simulation (using `eth_call` or similar). The transaction should NOT be executed on-chain.

### Recommended: Uniswap V3 path

* **QuoterV2**:
  * `quoteExactInputSingle(tokenIn, tokenOut, fee, amountIn, 0)` → `estimatedOut`
* **SwapRouter**:
  * `exactInputSingle(params)`
  * `amountOutMinimum = estimatedOut * (1 - slippage_bps/10000)`
  * `deadline = now + 600s` (example)
  * `recipient` from input
* **RPC**:
  * `eth_estimateGas(tx)` → gas estimate
  * `eth_call(tx)` → dry-run; catch reverts
* **Allowance note**:
  * **If **`from_token` is ERC-20, real on-chain execution requires `approve(router, amountIn)`.
  * **For ****simulation**, you may state this requirement (or use state overrides if available).

**Mainnet addresses (common)**

* **V3 QuoterV2**: `0x61fFE014bA17989E743c5F6cB21bF9697530B21e`
* **V3 SwapRouter**: `0xE592427A0AEce92De3Edee1F18E0157C05861564`
* **V2 Router02**: `0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D`

### MCP I/O (JSON Schemas)

**Input**

```
{"type":"object","properties":{
  "from_token":{"type":"string"},
  "to_token":{"type":"string"},
  "amount_in_wei":{"type":"string"},
  "slippage_bps":{"type":"integer"},
  "fee":{"type":"integer"},
  "recipient":{"type":"string"}
},"required":["from_token","to_token","amount_in_wei"]}
```

**Output**

```
{"type":"object","properties":{
  "amount_out_estimate":{"type":"string"},
  "gas_estimate":{"type":"string"},
  "calldata_hex":{"type":"string"}
},"required":["amount_out_estimate","gas_estimate","calldata_hex"]}
```

### Typical Failures

* **No liquidity at chosen fee tier → Quoter revert / call failure**
* **Insufficient allowance (for real execution) → explain requirement**
* **Amount too small / slippage too tight → Router revert**
* **Wrong fee tier (500/3000/10000) → try alternatives or keep a simple map**

---

## 7) MCP Runtime Integration

* **Register the three tools with ****rmcp**, including JSON-Schema I/O.
* **Handlers capture a shared **`EthCtx` (provider, signer).
* **Run with **`server.run_stdio().await` (Host communicates via stdio).
* **Write logs to ****stderr** (stdout is reserved for JSON-RPC payloads).

---

## 8) Logging, Errors, Precision

* **tracing**:
  * `INFO` — tool calls (redact sensitive values)
  * `DEBUG` — RPC method + latency (optional)
  * `WARN/ERROR` — upstream errors, throttling, contract call errors
* **Errors**: use `anyhow`/`thiserror`; surface as JSON-RPC error objects.
* **Precision**: keep raw values as `U256`; compute/format with **`rust_decimal`**; never use `f64` for money. Ensure all layers accept and emit decimal-safe types.

---

## 9) Tests (minimum)

1. **Balance (ETH)** — known address; assert `decimals=18`, formatted parses.
2. **Balance (ERC-20)** — USDC; assert `decimals=6`, formatting correct.
3. **Swap simulation (V3)** — WETH→USDC small `amountIn`, fee=3000; expect non-empty `amount_out_estimate`, `gas_estimate > 0`, non-empty `calldata_hex`.

**Note: tests rely on real RPC; add small retry or mark as integration.**

---

## 10) README (template essentials)

**Setup**

```
rustup default stable
cp .env.example .env   # fill ETH_RPC_URL, PRIVATE_KEY, DEFAULT_CHAIN_ID=1
cargo build --release
```

**Run (MCP via stdio)**

```
cargo run --release
```

**Tools**

* `get_balance(address, token?)`
* `get_token_price(token, quote=USD|ETH)` (Chainlink first, DEX fallback)
* `swap_tokens(from_token, to_token, amount_in_wei, slippage_bps=100, fee=3000, recipient=0x...)` (simulate)

**Example (JSON-RPC over stdio)**

```
{"jsonrpc":"2.0","id":1,"method":"get_balance","params":{"address":"0x..."}}
```

**Design Decisions (3–5)**

* **Price policy**: Chainlink-first for stability; DEX fallback for coverage; document limitations.
* **Swap**: QuoterV2 for price; build real Router calldata; simulate with `eth_call`/`eth_estimateGas` (no broadcast).
* **Precision**: `rust_decimal` with decimal-string I/O; avoid float errors.
* **MCP**: stdio JSON-RPC integration; logs via `tracing` to stderr.

**Known Limitations**

* **DEX fallback may be manipulable on thin pools; treat as approximation.**
* **No multi-hop/route optimization (single-hop + fixed fee by default).**
* **Real execution would require **`approve` for ERC-20; this assignment only simulates.

---

## 11) Security & Privacy

* **Read **`PRIVATE_KEY` only if signing; never log or commit it.
* `eth_call`/reads cost no gas and do not mutate state.
* **Do not return sensitive data (private keys, full signatures) in responses.**

---

## 12) Implementation Checklist (for agents)

* ** Init ****Tokio** + **tracing**
* `config.rs`: load `ETH_RPC_URL` / `PRIVATE_KEY` / `DEFAULT_CHAIN_ID`
* `eth_client.rs`: `Provider<Http>`, `SignerMiddleware`, ERC-20 `abigen!`
* `handlers/balance.rs`: ETH vs ERC-20 branches + `rust_decimal` formatting
* `types.rs`: shared structs
* `handlers/price.rs`: Chainlink Aggregator (latestRoundData/decimals) + DEX fallback
* `handlers/swap.rs`: QuoterV2 → Router calldata → `eth_estimateGas` + `eth_call`
* `mcp.rs`: register 3 tools (schemas + handlers); `run_stdio()`
* `tests/`: minimal integration tests
* `README`: usage, examples, decisions, limits

---

## 13) Mainnet Reference Addresses

* **Uniswap V3**

  * **QuoterV2 — **`0x61fFE014bA17989E743c5F6cB21bF9697530B21e`
  * **SwapRouter — **`0xE592427A0AEce92De3Edee1F18E0157C05861564`
* **Uniswap V2**

  * **Router02 — **`0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D`
* **Chainlink (examples)**

  * **ETH/USD — **`0x5f4ec3df9cbd43714fe2740f5e3616155c5b8419`
  * **USDC/USD — **`0x8fffffd4afb6115b954bd326cbe7b4ba576818f6`

  > **Verify with official docs; maintain a **`symbol → feed` map in code.
  >

---

## 14) Glossary

* **MCP** — Model Context Protocol; stdio-based JSON-RPC between host and tool process
* **eth_call** — read-only simulation; no gas; no state change
* **QuoterV2** — Uniswap V3 read-only quoting contract for `amountOut`
* **Concentrated Liquidity (V3)** — LPs provide liquidity within a price range; deeper liquidity near current price ⇒ smaller slippage
* **decimals** — ERC-20 fractional scale; human value = `raw / 10^decimals`
* **slippage_bps** — slippage in basis points; `100` = 1%

---

### End

**This ****agent.md** enables a code-gen agent to scaffold and implement a compliant MCP server that:**
** connects to a real RPC · reads real on-chain data · uses Chainlink-first pricing · builds real Uniswap transactions and simulates them with RPC (no broadcast).
