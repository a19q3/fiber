use crate::invoice::InvoiceBuilder;
use crate::rpc::info::NodeInfoResult;
use crate::rpc::invoice::ParseInvoiceResult;
use crate::rpc::payment::{GetPaymentCommandResult, SendPaymentCommandParams};
use anyhow::{anyhow, bail, Context, Result};
use ckb_jsonrpc_types::Script;
use fiber_types::invoice::Currency;
use fiber_types::{Hash256, Privkey};
use jsonrpsee::core::client::ClientT as _;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use jsonrpsee::rpc_params;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::str::FromStr as _;

pub const X402_VERSION: u32 = 2;
pub const FIBER_NETWORK: &str = "fiber";
pub const PAYMENT_RESPONSE_HEADER: &str = "PAYMENT-RESPONSE";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SupportedKind {
    pub x402_version: u32,
    pub scheme: String,
    pub network: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SupportedResponse {
    pub kinds: Vec<SupportedKind>,
    pub extensions: Vec<String>,
    pub signers: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequirements {
    pub scheme: String,
    pub network: String,
    pub amount: String,
    pub asset: String,
    pub pay_to: String,
    pub max_timeout_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Map<String, Value>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PaymentPayload {
    pub x402_version: u32,
    pub accepted: PaymentRequirements,
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Map<String, Value>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FacilitatorRequest {
    pub x402_version: u32,
    pub payment_payload: PaymentPayload,
    pub payment_requirements: PaymentRequirements,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SettlementResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payer: Option<String>,
    pub transaction: String,
    pub network: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Map<String, Value>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FiberSettlePayload {
    pub payment_method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipient_pubkey: Option<String>,
}

pub trait FiberRpc: Send + Sync {
    #[allow(async_fn_in_trait)]
    async fn node_info(&self) -> Result<NodeInfoResult>;
    #[allow(async_fn_in_trait)]
    async fn parse_invoice(&self, invoice: &str) -> Result<ParseInvoiceResult>;
    #[allow(async_fn_in_trait)]
    async fn send_payment(&self, params: SendPaymentCommandParams) -> Result<GetPaymentCommandResult>;
}

#[derive(Clone)]
pub struct FiberRpcClient {
    client: HttpClient,
}

impl FiberRpcClient {
    pub fn try_new(url: &str) -> Result<Self> {
        let client = HttpClientBuilder::default().build(url)?;
        Ok(Self { client })
    }
}

impl FiberRpc for FiberRpcClient {
    async fn node_info(&self) -> Result<NodeInfoResult> {
        self.client
            .request::<NodeInfoResult, _>("node_info", rpc_params![])
            .await
            .map_err(Into::into)
    }

    async fn parse_invoice(&self, invoice: &str) -> Result<ParseInvoiceResult> {
        self.client
            .request::<ParseInvoiceResult, _>(
                "parse_invoice",
                rpc_params![serde_json::json!({ "invoice": invoice })],
            )
            .await
            .map_err(Into::into)
    }

    async fn send_payment(&self, params: SendPaymentCommandParams) -> Result<GetPaymentCommandResult> {
        self.client
            .request::<GetPaymentCommandResult, _>("send_payment", rpc_params![params])
            .await
            .map_err(Into::into)
    }
}

pub fn supported_response() -> SupportedResponse {
    SupportedResponse {
        kinds: vec![SupportedKind {
            x402_version: X402_VERSION,
            scheme: "exact".to_string(),
            network: FIBER_NETWORK.to_string(),
            extra: None,
        }],
        extensions: Vec::new(),
        signers: Map::new(),
    }
}

pub async fn settle_with_rpc<R: FiberRpc>(rpc: &R, request: FacilitatorRequest) -> Result<SettlementResponse> {
    validate_request(&request)?;
    let payer = rpc.node_info().await?.pubkey.to_string();
    let payload: FiberSettlePayload = serde_json::from_value(request.payment_payload.payload.clone())
        .context("invalid Fiber payment payload")?;

    match payload.payment_method.as_str() {
        "invoice" => settle_invoice(rpc, &request, &payload, payer).await,
        "keysend" => settle_keysend(rpc, &request, &payload, payer).await,
        other => bail!("unsupported payment method: {other}"),
    }
}

fn validate_request(request: &FacilitatorRequest) -> Result<()> {
    if request.x402_version != X402_VERSION || request.payment_payload.x402_version != X402_VERSION {
        bail!("invalid_x402_version");
    }
    if request.payment_payload.accepted != request.payment_requirements {
        bail!("invalid_payment_requirements");
    }
    if request.payment_requirements.scheme != "exact" {
        bail!("unsupported_scheme");
    }
    if request.payment_requirements.network != FIBER_NETWORK {
        bail!("invalid_network");
    }
    Ok(())
}

async fn settle_invoice<R: FiberRpc>(
    rpc: &R,
    request: &FacilitatorRequest,
    payload: &FiberSettlePayload,
    payer: String,
) -> Result<SettlementResponse> {
    let invoice = payload.invoice.as_ref().ok_or_else(|| anyhow!("invalid_payload"))?;
    if request.payment_requirements.pay_to != *invoice {
        bail!("invalid_exact_fiber_payload_recipient_mismatch");
    }

    let parsed = rpc.parse_invoice(invoice).await?;
    let parsed_amount = parsed
        .invoice
        .amount
        .ok_or_else(|| anyhow!("invalid_exact_fiber_payload_amount_missing"))?;
    if parsed_amount.to_string() != request.payment_requirements.amount {
        bail!("invalid_exact_fiber_payload_amount_mismatch");
    }

    if request.payment_requirements.asset == "ckb" {
        if parsed.invoice.data.attrs.iter().any(|attr| matches!(attr, fiber_json_types::invoice::Attribute::UdtScript(_))) {
            bail!("invalid_exact_fiber_payload_asset_mismatch");
        }
    } else {
        let expected_script = request
            .payment_requirements
            .extra
            .as_ref()
            .and_then(|extra| extra.get("udtTypeScript"))
            .ok_or_else(|| anyhow!("invalid_exact_fiber_payload_asset_mismatch"))?;
        let invoice_script = parsed
            .invoice
            .data
            .attrs
            .iter()
            .find_map(|attr| match attr {
                fiber_json_types::invoice::Attribute::UdtScript(script) => Some(script),
                _ => None,
            })
            .ok_or_else(|| anyhow!("invalid_exact_fiber_payload_asset_mismatch"))?;
        if serde_json::to_value(invoice_script)? != *expected_script {
            bail!("invalid_exact_fiber_payload_asset_mismatch");
        }
    }

    let send_result = rpc
        .send_payment(SendPaymentCommandParams {
            target_pubkey: None,
            amount: None,
            payment_hash: None,
            final_tlc_expiry_delta: None,
            tlc_expiry_limit: None,
            invoice: Some(invoice.clone()),
            timeout: Some(request.payment_requirements.max_timeout_seconds),
            max_fee_amount: None,
            max_fee_rate: None,
            max_parts: None,
            trampoline_hops: None,
            keysend: None,
            udt_type_script: None,
            allow_self_payment: None,
            custom_records: None,
            hop_hints: None,
            dry_run: None,
        })
        .await?;

    Ok(payment_result_to_settlement(
        &request.payment_requirements,
        payer,
        send_result,
        Some(parsed.invoice.data.payment_hash.to_string()),
    ))
}

async fn settle_keysend<R: FiberRpc>(
    rpc: &R,
    request: &FacilitatorRequest,
    payload: &FiberSettlePayload,
    payer: String,
) -> Result<SettlementResponse> {
    let recipient = payload
        .recipient_pubkey
        .as_ref()
        .ok_or_else(|| anyhow!("invalid_payload"))?;
    if request.payment_requirements.pay_to != *recipient {
        bail!("invalid_exact_fiber_payload_recipient_mismatch");
    }

    let target_pubkey = fiber_json_types::Pubkey::from_str(recipient)
        .map_err(|_| anyhow!("invalid_exact_fiber_payload_recipient_mismatch"))?;
    let amount = request
        .payment_requirements
        .amount
        .parse::<u128>()
        .map_err(|_| anyhow!("invalid_payment_requirements"))?;

    let udt_type_script = match request.payment_requirements.asset.as_str() {
        "ckb" => None,
        _ => Some(extract_udt_script(request)?),
    };

    let send_result = rpc
        .send_payment(SendPaymentCommandParams {
            target_pubkey: Some(target_pubkey),
            amount: Some(amount),
            payment_hash: None,
            final_tlc_expiry_delta: None,
            tlc_expiry_limit: None,
            invoice: None,
            timeout: Some(request.payment_requirements.max_timeout_seconds),
            max_fee_amount: None,
            max_fee_rate: None,
            max_parts: None,
            trampoline_hops: None,
            keysend: Some(true),
            udt_type_script,
            allow_self_payment: None,
            custom_records: None,
            hop_hints: None,
            dry_run: None,
        })
        .await?;

    Ok(payment_result_to_settlement(
        &request.payment_requirements,
        payer,
        send_result,
        None,
    ))
}

fn extract_udt_script(request: &FacilitatorRequest) -> Result<Script> {
    let value = request
        .payment_requirements
        .extra
        .as_ref()
        .and_then(|extra| extra.get("udtTypeScript"))
        .ok_or_else(|| anyhow!("invalid_exact_fiber_payload_asset_mismatch"))?
        .clone();
    serde_json::from_value(value).map_err(Into::into)
}

fn payment_result_to_settlement(
    requirements: &PaymentRequirements,
    payer: String,
    result: GetPaymentCommandResult,
    transaction_override: Option<String>,
) -> SettlementResponse {
    let success = matches!(result.status, fiber_json_types::payment::PaymentStatus::Success);
    let transaction = if success {
        transaction_override.unwrap_or_else(|| result.payment_hash.to_string())
    } else {
        String::new()
    };
    SettlementResponse {
        success,
        error_reason: if success {
            None
        } else {
            Some(
                result
                    .failed_error
                    .unwrap_or_else(|| "unexpected_settle_error".to_string()),
            )
        },
        payer: Some(payer),
        transaction,
        network: requirements.network.clone(),
        amount: Some(requirements.amount.clone()),
        extensions: None,
    }
}

pub fn build_invoice_for_test(amount: u128, payee: Privkey) -> String {
    let pubkey = payee.pubkey();
    InvoiceBuilder::new(Currency::Fibb)
        .amount(Some(amount))
        .payee_pub_key(pubkey.into())
        .payment_hash(Hash256::from([3u8; 32]))
        .build_with_sign(|message| secp256k1::SECP256K1.sign_ecdsa_recoverable(message, &payee.0))
        .expect("build signed invoice")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fiber_json_types::graph::UdtCfgInfos;
    use fiber_json_types::payment::{GetPaymentCommandResult, PaymentStatus};
    use fiber_types::sample::deterministic_privkey;

    struct StubFiberRpc {
        node_info: NodeInfoResult,
        parsed_invoice: Option<ParseInvoiceResult>,
        send_payment_result: GetPaymentCommandResult,
        last_send_payment: std::sync::Mutex<Option<SendPaymentCommandParams>>,
    }

    impl StubFiberRpc {
        fn new(invoice: Option<String>, status: PaymentStatus, failed_error: Option<&str>) -> Self {
            let node_privkey = deterministic_privkey(42, 1);
            let node_info = NodeInfoResult {
                version: "test".to_string(),
                commit_hash: "test".to_string(),
                pubkey: node_privkey.pubkey().into(),
                features: Vec::new(),
                node_name: None,
                addresses: Vec::new(),
                chain_hash: fiber_json_types::Hash256([0u8; 32]),
                open_channel_auto_accept_min_ckb_funding_amount: 0,
                auto_accept_channel_ckb_funding_amount: 0,
                default_funding_lock_script: Default::default(),
                tlc_expiry_delta: 0,
                tlc_min_value: 0,
                tlc_fee_proportional_millionths: 0,
                channel_count: 0,
                pending_channel_count: 0,
                peers_count: 0,
                udt_cfg_infos: UdtCfgInfos(Vec::new()),
            };

            let parsed_invoice = invoice.map(|invoice| ParseInvoiceResult {
                invoice: invoice.parse::<fiber_types::invoice::CkbInvoice>().unwrap().into(),
            });

            Self {
                node_info,
                parsed_invoice,
                send_payment_result: GetPaymentCommandResult {
                    payment_hash: fiber_json_types::Hash256([9u8; 32]),
                    status,
                    created_at: 1,
                    last_updated_at: 2,
                    failed_error: failed_error.map(str::to_string),
                    fee: 7,
                    custom_records: None,
                    #[cfg(debug_assertions)]
                    routers: Vec::new(),
                },
                last_send_payment: std::sync::Mutex::new(None),
            }
        }
    }

    impl FiberRpc for StubFiberRpc {
        async fn node_info(&self) -> Result<NodeInfoResult> {
            Ok(self.node_info.clone())
        }

        async fn parse_invoice(&self, _invoice: &str) -> Result<ParseInvoiceResult> {
            self.parsed_invoice.clone().ok_or_else(|| anyhow!("missing invoice"))
        }

        async fn send_payment(&self, params: SendPaymentCommandParams) -> Result<GetPaymentCommandResult> {
            *self.last_send_payment.lock().unwrap() = Some(params);
            Ok(self.send_payment_result.clone())
        }
    }

    fn invoice_request(invoice: String) -> FacilitatorRequest {
        let requirements = PaymentRequirements {
            scheme: "exact".to_string(),
            network: FIBER_NETWORK.to_string(),
            amount: "1000".to_string(),
            asset: "ckb".to_string(),
            pay_to: invoice.clone(),
            max_timeout_seconds: 30,
            extra: Some(Map::new()),
        };
        FacilitatorRequest {
            x402_version: X402_VERSION,
            payment_payload: PaymentPayload {
                x402_version: X402_VERSION,
                accepted: requirements.clone(),
                payload: serde_json::to_value(FiberSettlePayload {
                    payment_method: "invoice".to_string(),
                    invoice: Some(invoice),
                    payment_hash: None,
                    recipient_pubkey: None,
                })
                .unwrap(),
                resource: None,
                extensions: None,
            },
            payment_requirements: requirements,
        }
    }

    fn keysend_request(recipient_pubkey: String) -> FacilitatorRequest {
        let requirements = PaymentRequirements {
            scheme: "exact".to_string(),
            network: FIBER_NETWORK.to_string(),
            amount: "1000".to_string(),
            asset: "ckb".to_string(),
            pay_to: recipient_pubkey.clone(),
            max_timeout_seconds: 30,
            extra: Some(Map::new()),
        };
        FacilitatorRequest {
            x402_version: X402_VERSION,
            payment_payload: PaymentPayload {
                x402_version: X402_VERSION,
                accepted: requirements.clone(),
                payload: serde_json::to_value(FiberSettlePayload {
                    payment_method: "keysend".to_string(),
                    invoice: None,
                    payment_hash: None,
                    recipient_pubkey: Some(recipient_pubkey),
                })
                .unwrap(),
                resource: None,
                extensions: None,
            },
            payment_requirements: requirements,
        }
    }

    #[tokio::test]
    async fn settle_invoice_maps_successful_payment_to_settlement_response() {
        let invoice = build_invoice_for_test(1000, deterministic_privkey(7, 1));
        let rpc = StubFiberRpc::new(Some(invoice.clone()), PaymentStatus::Success, None);
        let invoice_hash = invoice
            .parse::<fiber_types::invoice::CkbInvoice>()
            .unwrap()
            .payment_hash()
            .to_string()
            .trim_start_matches("Hash256(")
            .trim_end_matches(')')
            .to_string();

        let response = settle_with_rpc(&rpc, invoice_request(invoice.clone())).await.unwrap();

        assert!(response.success);
        assert_eq!(response.network, FIBER_NETWORK);
        assert_eq!(response.amount.as_deref(), Some("1000"));
        assert_eq!(response.transaction, invoice_hash);
        let sent = rpc.last_send_payment.lock().unwrap().clone().unwrap();
        assert_eq!(sent.invoice, Some(invoice));
        assert_eq!(sent.keysend, None);
    }

    #[tokio::test]
    async fn settle_keysend_maps_recipient_and_amount_into_send_payment() {
        let recipient = hex::encode(deterministic_privkey(8, 1).pubkey().0);
        let rpc = StubFiberRpc::new(None, PaymentStatus::Success, None);

        let response = settle_with_rpc(&rpc, keysend_request(recipient.clone())).await.unwrap();

        assert!(response.success);
        let sent = rpc.last_send_payment.lock().unwrap().clone().unwrap();
        assert_eq!(sent.amount, Some(1000));
        assert_eq!(sent.keysend, Some(true));
        assert_eq!(sent.target_pubkey.unwrap().to_string(), recipient);
    }

    #[tokio::test]
    async fn settle_returns_graceful_failure_for_failed_payment() {
        let invoice = build_invoice_for_test(1000, deterministic_privkey(9, 1));
        let rpc = StubFiberRpc::new(Some(invoice.clone()), PaymentStatus::Failed, Some("insufficient_funds"));

        let response = settle_with_rpc(&rpc, invoice_request(invoice)).await.unwrap();

        assert!(!response.success);
        assert_eq!(response.error_reason.as_deref(), Some("insufficient_funds"));
        assert_eq!(response.transaction, "");
    }
}
