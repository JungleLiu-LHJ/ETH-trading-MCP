# WalletMcp — Ethereum Trading MCP Server

**Stdio JSON‑RPC server exposing three tools for Ethereum mainnet:**

* `get_balance` — ETH or ERC‑20 balance lookup
* `get_token_price` — Chainlink‑first price with Uniswap V3 fallback
* `swap_tokens` — Build real Uniswap V3 calldata and simulate (no broadcast)

## Design Decisions

#### Code structure

* **Three layers: MCP (stdio/JSON‑RPC) → Service (orchestration) → Implementations (balance/price/swap). This isolates concerns, keeps handlers thin, and makes core logic testable without I/O.**
* **Shared context (**`ServiceContext`) holds `provider`, and a token `registry` behind `Arc`/`RwLock`. This enables safe concurrent reads with occasional writes when discovering new tokens.
* **JSON‑RPC over stdio keeps the binary host‑agnostic and MCP‑compatible; stdout is reserved for protocol payloads, logs go to stderr via **`tracing`.

#### Core Flows

* **Token metadata and registry**
  * **Defaults metadata are saved in (**`config/token_defaults.json`) for deterministic behavior and quick startup.
  * **On‑demand discovery: if a token isn’t in the registry but an address is provided, the server fetches minimal ERC‑20 metadata and caches it, avoiding a hard dependency on static config.**
  * **I decoupled the registry and main flow to facilitate later maintenance.**
* **Pricing policy**
  * **Firstly request prices from Chainlink for integrity and resilience**
  * **If not exist, then falls back to Uniswap V3 Quoter**
* **Swap simulation by construction**
  * **Call Uniswap V3 QuoterV2  a single-hop output, then calculate the slippage.**
  * **Build the exact SwapRouter calldata send on-chain, then simulate it via the node (eth_estimateGas + eth_call) using the private key**

## Setup

* **Dependencies**
  * **Rust (stable) and Cargo**
  * **An HTTPS Ethereum RPC endpoint (e.g., Alchemy/Infura/Public)**
* **Clone and build**
  * `cargo build --release`
* **Configuration**
  * **Option A: environment variables (dotenv supported)**
    * `ETH_RPC_URL` — HTTPS RPC URL (required)
    * `PRIVATE_KEY` — hex private key, with or without `0x` (optional; required for swap simulation)
    * `DEFAULT_CHAIN_ID` — defaults to `1` (mainnet)
  * **Option B: **`Config.toml` (preferred in production). Example:
    ```
    eth_rpc_url = "https://..."
    private_key = "0xabc..."
    default_chain_id = 1
    ```
* **Token registry defaults**
  * **in **`config/token_defaults.json` (symbols, addresses, decimals, Chainlink feeds, default Uniswap fee tiers).

---

## Run

* **Start the MCP server over stdio (reads JSON‑RPC lines from stdin, writes responses to stdout):**
  * `cargo run --release`
* **Or run the compiled binary:**
  * `target/release/walletmcp`
* **The server logs to stderr via **`tracing`; stdout is reserved for JSON‑RPC payloads.

---

## Example MCP Calls (JSON‑RPC over stdio)

* **Get balance **
  * **Parameters**

    * `address`: holder address (checksummed or lowercased `0x...`).
    * `token` (optional): ERC‑20 address or known symbol; omit for ETH.
  * **ETH**

    * **Request:**

    ```
    {"jsonrpc":"2.0","id":"1","method":"get_balance","params":{"address":"0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"}}
    ```

    * **Example response:**

    ```
    {"jsonrpc":"2.0","result":{"decimals":18,"formatted":"3.758181447334319014","raw":"3758181447334319014","symbol":"ETH"},"id":"1"}
    ```
  * **USDT**

    * **Request:**

    ```
    {"jsonrpc":"2.0","id":"bal-usdt","method":"get_balance","params":{"address":"0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045","token":"0xdAC17F958D2ee523a2206206994597C13D831ec7"}}

    ```

    * **Example response:**

    ```
    {"jsonrpc":"2.0","id":"bal-usdc","method":"get_balance","params":{"address":"0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045","token":"USDC"}}

    ```
* **Get token price **
  * **Parameters**

    * `base`: token to price (address or symbol).
    * `quote` (optional): `"USD"` or `"ETH"` (defaults to `"USD"`).
  * **USDC → USD**

    * **Request:**

    ```
    {"jsonrpc":"2.0","id":"price-1","method":"get_token_price","params":{"base":"USDC"}}
    ```

    * **response:**

    ```
    {"jsonrpc":"2.0","result":{"base":"USDC","decimals":8,"price":"0.99981815","quote":"USD","source":"chainlink"},"id":"price-1"}
    ```
* **Simulate swap **
  * **Parameters**
    * `from_token` / `to_token`: address or known symbol; symbols resolve via the registry.
    * `amount_in_wei`: input amount as a decimal string in wei.
    * `slippage_bps` (optional): basis points tolerance (default 100 = 1%).
    * `fee` (optional): Uniswap V3 fee tier.
    * `recipient` (optional): output receiver; defaults to the signer address.
    * `sqrt_price_limit` (optional): X96 price boundary; `"0"` or omit for no limit.
  * **Request:**
    ```
    {"jsonrpc":"2.0","id":"swap-1","method":"swap_tokens","params":{"from_token":"....","to_token":"0x6B175474E89094C44Da98b954EedeAC495271d0F","amount_in_wei":"10000000000000","slippage_bps":100,"fee":3000,"recipient":"0xYourAddressHere"}}
    ```
  * **Example response:**
    ```
    {"jsonrpc":"2.0","result":{"amount_out_estimate":"0.033810900284009015","amount_out_min":"0.033472791281168924","calldata_hex":"0x414bf389000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000006b175474e89094c44da98b954eedeac495271d0f0000000000000000000000000000000000000000000000000000000000000bb80000000000000000000000004fc74ba63ddd9c685f8bb59f25d2c9345d3c72e600000000000000000000000000000000000000000000000000000000690c4e19000000000000000000000000000000000000000000000000000009184e72a0000000000000000000000000000000000000000000000000000076eb5389f4721c0000000000000000000000000000000000000000000000000000000000000000","gas_estimate":"121109","router":"0xe592427a0aece92de3edee1f18e0157c05861564"},"id":"swap-1"}
    ```

**Notes**

* **The exact numbers vary with market conditions and RPC provider state.**
* **Swap simulation requires **`PRIVATE_KEY` to be configured to derive a sender and build a realistic transaction context.

**Descriptions**

* `get_balance`
  * **Params**
    * `address` string — holder address (`0x` + 40 hex chars).
    * `token` string|null — optional ERC‑20 address or known symbol (per `config/token_defaults.json`). Omit to fetch native ETH balance.
  * **Returns **`BalanceOut` — `{ symbol, raw, decimals, formatted }` where `formatted = raw / 10^decimals`.
  * **Errors — invalid address/symbol, RPC failures.**
* `get_token_price`
  * **Params**
    * `base` string — token address or symbol (known to the registry or discoverable via on‑chain ERC‑20 metadata).
    * `quote` string (optional, default `"USD"`) — one of `"USD"` or `"ETH"`.
  * **Returns **`PriceOut` — `{ base, quote, price, source, decimals }` where `source` is `chainlink`, `chainlink (via USD/ETH)`, or `uniswap_v3 (fee N)`.
  * **Notes — Chainlink first; falls back to Uniswap V3 Quoter using default fee from the token registry.**
  * **Errors — unsupported token, missing quote token configuration, RPC failures.**
* `swap_tokens`
  * **Params**
    * `from_token`/`to_token` string — address or known symbol.
    * `amount_in_wei` string — decimal string of input amount in wei.
    * `slippage_bps` integer (default `100`) — basis points (max `10000`).
    * `fee` integer (default `3000`) — Uniswap V3 fee tier (e.g., 500 / 3000 / 10000).
    * `recipient` string (optional) — address to receive output; defaults to signer address.
    * `sqrt_price_limit` string (optional, advanced) — raw `X96` limit; omit for no limit.
  * **Returns **`SwapSimOut` — `{ amount_out_estimate, amount_out_min, gas_estimate, calldata_hex, router }`.
  * **Requirements — **`PRIVATE_KEY` must be configured to derive a sender for realistic calldata and gas estimation.
  * **Errors — invalid numeric input, slippage > 10000, quote returned 0, gas estimation/eth_call failures, RPC issues.**

**Error Codes**

* `-32602` invalid params; `-32601` method not found; `-32603` internal/serialization.
* `-32001` config; `-32002` RPC; `-32010` price; `-32020` swap; `-32030` wallet; `-32040` I/O.

---

## Known Limitations & Assumptions

* **registry are static and did not update**
* **Single‑hop Uniswap V3 only; no route search or multi‑hop paths.**
* **Fallback DEX prices can be noisy/manipulable on thin liquidity pools; treat as indicative.**
* **Limited Chainlink coverage: only feeds listed in **`token_defaults.json` (unless extended in code).
* **Real execution would require ERC‑20 approvals and balances; this project only simulates and never broadcasts.**
* **Mainnet contract addresses are hard‑coded for Uniswap V3 QuoterV2 and SwapRouter.**

---

## Core Function Logic (high‑level)

**get_balance (balance lookup)**

* **Accepts an address and an optional token identifier (address or symbol).**
* **If no token is provided, read the native ETH balance and format using 18 decimals.**
* **If a token is provided, resolve symbols via the in‑memory registry (or use the given address), fetch minimal token metadata as needed, read the holder’s token balance, and format using that token’s decimals.**
* **Returns symbol, raw amount, decimals, and a human‑readable formatted string. Invalid inputs or network issues surface as errors.**

**get_token_price (pricing)**

* **Resolve the base asset (address or symbol). If it’s not already known, discover minimal metadata on chain and add it to the registry cache; if it still can’t be recognized, return “unsupported token”.**
* **Apply a Chainlink‑first policy:**
  * **If a direct base/quote oracle feed exists, use it.**
  * **If not, try to pivot via USD or ETH using well‑known feeds (base/USD + WETH/USD or base/ETH + WETH/USD).**
* **If no oracle path is available, fall back to a single‑hop DEX spot quote using a default fee tier and representative quote tokens (USDC for USD, WETH for ETH).**
* **Return a decimal string price with a source label. If required quote‑token configuration is missing, return an error.**

**swap_tokens (simulation, no broadcast)**

* **Resolve input and output assets and require a signer to build a realistic transaction context (sender and execution domain).**
* **Validate amount and slippage; default the recipient to the signer when not supplied.**
* **Obtain a read‑only output estimate for the chosen fee tier.**
* **Derive a minimum acceptable output from the slippage tolerance and assemble real router calldata with a deadline and recipient.**
* **Estimate gas and dry‑run the transaction; format outputs using the output token’s decimals.**
* **Return the estimated output, minimum output, gas estimate, calldata, and router address. Real execution would require allowances and sufficient balances; this tool only simulates.**

---

## Network Calls & Data Sources

* **get_balance**
  * **Ethereum RPC only.**
  * **ETH: **`eth_getBalance(address, latest)`.
  * **ERC‑20: **`eth_call` to token contract for `balanceOf(address)`, plus optional metadata reads `decimals()` and `symbol()` for formatting.
* **get_token_price**
  * **Ethereum RPC + on‑chain data sources.**
  * **Chainlink (preferred): **`eth_call` to AggregatorV3 contracts for `decimals()` and `latestRoundData()` on mainnet feeds (e.g., WETH/USD, USDC/USD).
  * **Pivoting: combines two Chainlink feeds (base/USD with WETH/USD, or base/ETH with WETH/USD) when a direct feed is missing.**
  * **Uniswap V3 fallback: **`eth_call` to QuoterV2 at `0x61fFE014bA17989E743c5F6cB21bF9697530B21e` using `quoteExactInputSingle(...)` for a single‑hop spot quote.
  * **Registry ensure step (as needed): **`eth_call` to the token contract for `decimals()`/`symbol()` when a token is first seen.
* **swap_tokens (simulation)**
  * **Ethereum RPC + Uniswap V3 contracts.**
  * **Quote: **`eth_call` to Uniswap QuoterV2 for a single‑hop output estimate.
  * **Calldata: build Uniswap V3 SwapRouter **`exactInputSingle(...)` transaction targeting `0xE592427A0AEce92De3Edee1F18E0157C05861564`.
  * **Simulation: **`eth_estimateGas` for the router transaction, then `eth_call` to dry‑run it; no `eth_sendRawTransaction` (never broadcasts).
  * **Metadata: if needed, **`eth_call` to token contracts for decimals to format output amounts.

## Testing

**Remiding: some unit-test need complete Configurations（ETH_RPC_URL，PRIVATE_KEY)!!!**

* **Run fast unit tests: **`cargo test`
* **Live‑network tests are marked **`#[ignore]` and require env vars (see `tests/` and comments). Enable them manually if you have real RPC and funded keys.

---

## Security Notes

* **Never commit real private keys. Use environment variables locally and a secure secret manager in production.**
* **The server never broadcasts transactions; simulation uses **`eth_estimateGas` and `eth_call` only.
