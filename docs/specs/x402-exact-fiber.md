# x402 `exact` on `fiber`

## Summary

This document defines a proposed `(scheme = exact, network = fiber)` pair for the x402 ecosystem.

The goal is to describe how x402 could use the Fiber payment-channel network for fixed-price payments instead of an on-chain execution path. The pair is intended for instant, low-fee payments where a resource server and facilitator can verify that the selected Fiber payment details match the requested amount, recipient, and asset.

This document is a Fiber-side specification draft. It is not yet an accepted x402 scheme implementation, and several upstream compatibility questions remain open, especially around network identifiers and the facilitator trust model. The likely upstreaming path is described in [Acceptance into x402](#acceptance-into-x402).

## Design Goals

- Reuse x402's existing `exact` semantics: the client pays one exact amount for one resource access decision.
- Preserve as much of x402's trust-minimizing model as Fiber allows, while explicitly documenting where a Fiber facilitator-executed payment differs from signature-based chains.
- Fit Fiber's payment model, which supports invoice payments, keysend, payment hashes, and multi-asset routing through `udt_type_script`.
- Keep the x402 surface area small by using core fields where possible and `extra` for Fiber-specific metadata.

## Network Identifier

The provisional network string for this pair is `fiber`.

This identifier refers to the Fiber payment network as a payment rail, not to a single CKB chain ID. Network-specific deployment details such as mainnet/testnet/devnet invoice prefixes SHOULD be encoded inside Fiber addresses or in `extra` metadata when the facilitator needs to distinguish environments.

This is a draft convenience identifier, not a final upstream recommendation. x402 appears to key support and discovery off the `network` field, so an upstream proposal will likely need either a CAIP-2-compatible Fiber identifier or an explicit x402 exception for `fiber`.

## PaymentRequirements Mapping

For `(exact, fiber)`, `PaymentRequirements` can be modeled as follows during the design phase:

```json
{
  "scheme": "exact",
  "network": "fiber",
  "amount": "100000000",
  "asset": "ckb",
  "payTo": "fibt1000000001...",
  "maxTimeoutSeconds": 60,
  "extra": {
    "fiberNetwork": "testnet",
    "settlementMethod": "invoice",
    "invoice": "fibt1000000001...",
    "assetType": "native"
  }
}
```

### Field Semantics

- `scheme` MUST be `exact`.
- `network` MUST be whatever Fiber identifier upstream x402 accepts. This draft uses `fiber` as a placeholder.
- `amount` MUST be the exact Fiber transfer amount as an integer string in the asset's smallest unit.
  - For native CKB, the unit is `shannon`.
  - For UDT or RGB++ assets, the unit is the token's on-network minimal unit as understood by the Fiber node and the receiving invoice.
- `asset` identifies the transferred asset.
  - `ckb` SHOULD be used for native CKB.
  - For UDT or RGB++ assets, `asset` SHOULD be a canonical asset identifier agreed on by the facilitator and resource server.
  - A practical choice is the serialized `udt_type_script` or a deterministic digest of that script, with the full script carried in `extra.udtTypeScript`.
- `payTo` MUST identify the payment destination that the client is selecting.
  - For invoice-based settlement, `payTo` SHOULD be the raw Fiber invoice string.
  - For keysend-based settlement, `payTo` SHOULD be the destination Fiber node public key.
- `maxTimeoutSeconds` bounds how long the selected payment details are valid for facilitator-side verification and settlement.
- `extra` carries Fiber-specific data defined below.

### Required `extra` Fields

Implementations SHOULD support these `extra` fields:

- `settlementMethod`: `invoice` or `keysend`.
- `fiberNetwork`: one of `mainnet`, `testnet`, or `devnet` when the deployment needs explicit environment separation.
- `assetType`: `native` or `udt`.

For `settlementMethod = invoice`, `extra` MUST include:

- `invoice`: the Fiber invoice string.

For `assetType = udt`, `extra` MUST include:

- `udtTypeScript`: the full serialized UDT type script used by Fiber invoices and channels.

For `settlementMethod = keysend`, `extra` SHOULD include:

- `recipientPubkey`: the destination Fiber node public key if it is not already used in `payTo`.

## Fiber Payment Header Payload

The x402 payment payload for `(exact, fiber)` MUST carry enough information for a facilitator to verify and, if applicable, execute the Fiber payment without changing the recipient, asset, or amount.

Unlike EVM-style `exact` mechanisms, this draft does not yet define a client-signed spend authorization that the facilitator can submit independently. In the invoice flow, the facilitator is the party that actually originates the Fiber payment from a connected node. That makes the trust model materially different from existing signature-based x402 mechanisms and is an explicit topic for upstream review.

Recommended payload shape:

```json
{
  "x402Version": 2,
  "accepted": {
    "scheme": "exact",
    "network": "fiber",
    "amount": "100000000",
    "asset": "ckb",
    "payTo": "fibt1000000001...",
    "maxTimeoutSeconds": 60,
    "extra": {
      "fiberNetwork": "testnet",
      "settlementMethod": "invoice",
      "invoice": "fibt1000000001...",
      "assetType": "native"
    }
  },
  "payload": {
    "paymentMethod": "invoice",
    "invoice": "fibt1000000001...",
    "paymentHash": "0xafb604f74c28009732ed4c82983cf1efaddf62ee36442f360fb4a8c79b845432"
  }
}
```

### Payload Variants

#### Invoice Variant

For invoice settlement, the payload SHOULD contain:

- `paymentMethod = invoice`
- `invoice`
- `paymentHash`

The invoice commits to the payment hash and, when present, can commit to amount, asset metadata, and payee identity. In this variant, the x402 client is not supplying a separate transferable signature as in EVM-based flows. Instead, the client is selecting a Fiber invoice that exactly matches the `PaymentRequirements`, and the facilitator verifies and pays that invoice.

To make this safe for `exact`, the invoice variant SHOULD be limited to invoices that:

- contain an explicit non-zero amount,
- are not expired,
- include a payee public key,
- and include a signature.

#### Keysend Variant

For keysend settlement, the payload SHOULD contain:

- `paymentMethod = keysend`
- `recipientPubkey`
- `paymentPreimage` or `paymentHash`
- `udtTypeScript` when transferring a non-native asset

Because keysend lacks an invoice object that commits to destination and amount, the facilitator MUST treat the x402 payload as the complete authorization record and MUST reject requests unless the recipient and asset are fully specified.

## Verification

The facilitator MUST verify the following before returning success from `/verify`:

1. `accepted.scheme` is `exact` and `accepted.network` is the chosen Fiber network identifier.
2. The requested `amount` exactly matches the amount authorized by the Fiber payment data.
3. The destination matches `payTo`.
4. The asset matches `asset` and any Fiber-specific asset metadata in `extra`.
5. The selected payment details are still live under `maxTimeoutSeconds` and any Fiber invoice expiry.
6. The facilitator can route or otherwise expects to be able to settle the payment without mutating the economic terms.

### Invoice Verification Rules

For invoice payments, the facilitator MUST:

1. Parse `invoice`.
2. Verify the invoice contains an explicit amount and that it exactly equals `accepted.amount`.
3. Reject any amount-unspecified or zero-amount invoice.
4. Verify the invoice asset matches `accepted.asset`.
   - If the invoice has no `udt_script`, the asset MUST be treated as native CKB.
   - If the invoice includes `udt_script`, it MUST match `extra.udtTypeScript` when present.
5. Verify the invoice has not expired.
6. Verify the invoice includes both `payee_public_key` and `signature`.
7. Verify that `payee_public_key` is consistent with the intended payee.
8. Verify the invoice's `payment_hash` matches `payload.paymentHash` when both are present.

### Keysend Verification Rules

For keysend payments, the facilitator MUST:

1. Verify `recipientPubkey` or `payTo` identifies a single recipient node.
2. Verify the amount equals `accepted.amount` exactly.
3. Verify `udtTypeScript` is present when the asset is not native CKB.
4. Verify the facilitator's send request would route value only to the authorized recipient.
5. Reject the payment if the payload leaves any destination, asset, or amount ambiguity.

### Simulation and Capability Checks

Before returning a successful verification response, the facilitator SHOULD:

- Parse the invoice or proposed keysend request with a Fiber-compatible library.
- Confirm local node connectivity and route feasibility.
- Confirm that the facilitator controls the Fiber node or RPC credentials needed to settle.
- Re-check the same payment data immediately before `/settle`.

## Settlement

Settlement is performed by a Fiber-capable facilitator calling the Fiber payment APIs.

### Invoice Settlement

For invoice payments, the facilitator SHOULD call Fiber `send_payment` with the authorized `invoice` field.

The facilitator MUST NOT:

- substitute a different invoice,
- increase the amount,
- change the asset,
- or redirect payment to a different recipient.

The settlement result SHOULD record at least:

- `paymentHash`,
- final payment status,
- any Fiber fee charged to the payer-side facilitator node,
- and a timestamp or payment identifier suitable for receipt generation.

### Keysend Settlement

For keysend payments, the facilitator SHOULD call `send_payment` using the exact recipient, amount, and asset metadata authorized in the x402 payload.

If the implementation supports a lower-level router flow such as `build_router` plus `send_payment_with_router`, the routed payment MUST preserve the same recipient, asset, and amount.

## Multi-Asset Support

Fiber's advantage in x402 is native support for more than one asset family.

This pair SHOULD support at least:

- native CKB, and
- RGB++ or other UDT-backed assets that are represented in Fiber by `udt_type_script`.

### Asset Encoding Rules

- Native CKB MUST be encoded as `asset = ckb` and MUST NOT require `udtTypeScript`.
- UDT assets MUST include enough metadata to uniquely bind the payment to one asset class.
- `extra.udtTypeScript` is the authoritative asset descriptor for current Fiber integrations.
- If a shorter canonical asset identifier is introduced later in x402, it SHOULD resolve one-to-one to the same underlying `udt_type_script`.

### Stablecoin Considerations

For stablecoins issued as RGB++ or UDT assets, the x402 layer SHOULD treat them exactly like any other exact-value asset:

- `amount` is still an integer in minimal units.
- asset identity is still bound by `udtTypeScript`.
- no special x402 semantics are required beyond asset identification.

This keeps `(exact, fiber)` simple while still enabling stablecoin-denominated API pricing.

## Security Considerations

### Destination Integrity

The facilitator MUST be unable to redirect funds. Invoice settlement is preferred, but only when the invoice includes a payee public key and signature so recipient identity is actually bound.

### Exact-Amount Enforcement

The facilitator MUST reject any payment that does not transfer exactly `accepted.amount` in the requested asset. Underpayment and overpayment are both invalid for `exact`.

### Replay and Freshness

Invoice expiry and `maxTimeoutSeconds` SHOULD be used together to bound replay risk. Implementations SHOULD treat a paid, expired, or cancelled invoice as invalid for new x402 authorizations.

### Facilitator Risk

Unlike EVM signature-based flows where the client signs a spend authorization, Fiber settlement generally requires the facilitator to originate the actual network payment from a connected Fiber node. That means the facilitator bears routing, liquidity, and fee-exposure risk, and the overall trust model is weaker than a pure client-signed authorization flow. Implementations SHOULD re-verify invoice freshness and route feasibility immediately before settlement.

### Trust Model

This design is best understood as a facilitator-executed exact-payment rail, not yet as a drop-in equivalent of x402's existing signature-based `exact` mechanisms. It fits deployments where the facilitator is explicitly trusted to originate the Fiber payment or where the client has a prior off-protocol relationship with the facilitator. Whether that trust model is acceptable for upstream x402 is an open design question.

## Recommended Initial Scope

The simplest initial upstream proposal is:

- `scheme = exact`
- `network = fiber`
- invoice-only settlement in v1
- native CKB plus UDT assets via `extra.udtTypeScript`

Keysend MAY be specified later if x402 reviewers prefer the initial pair to rely only on invoice-shaped authorizations.

## Acceptance into x402

Based on x402's current contribution process, a likely path for acceptance is:

1. Open an issue or discussion in `x402-foundation/x402` to validate the trust model and network identifier direction.
2. Open a specification-only PR adding `specs/schemes/exact/scheme_exact_fiber.md`.
3. Discuss whether the final network identifier should be a single value or an environment-qualified family.
4. Get review from the x402 Foundation team on security properties, especially the trust model difference between Fiber facilitator settlement and signature-based chains.
5. If the spec is accepted, follow with one reference SDK implementation and the facilitator support required by that SDK.

This is directionally consistent with x402's documented process for new chain families: discuss first, merge a spec, then land one reference SDK implementation before expanding to other SDKs.

## Open Questions

1. What upstream-acceptable network identifier should represent Fiber environments?
2. Should upstream x402 require invoice-only support for the first version and defer keysend entirely?
3. Is Fiber's facilitator-executed trust model acceptable for `exact`, or does x402 require a stronger client-controlled authorization artifact first?
4. Should `asset` directly carry serialized `udt_type_script`, or should that stay in `extra` with a shorter canonical identifier in `asset`?
5. What receipt shape should `/settle` return so resource servers can reason about Fiber finality and payment fees?

## References

- `docs/specs/payment-invoice.md`
- `docs/specs/cross-chain-htlc.md`
- `docs/payment-lifecycle.md`
- `docs/biscuit-auth.md`
- x402 repository: <https://github.com/x402-foundation/x402>
- Fiber design issue: <https://github.com/nervosnetwork/fiber/issues/1255>
