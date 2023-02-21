use std::str::FromStr;

use actix::Addr;
use near_o11y::WithSpanContextExt;
use near_primitives::{types::BlockId, views::ExecutionOutcomeWithIdView};

use crate::models::{AccountIdentifier, FungibleTokenEvent};

pub(crate) async fn collect_nep141_events(
    receipt_execution_outcomes: &Vec<ExecutionOutcomeWithIdView>,
    block_header: &near_primitives::views::BlockHeaderView,
    view_client_addr: &Addr<near_client::ViewClientActor>,
) -> crate::errors::Result<Vec<FungibleTokenEvent>> {
    let mut res = Vec::new();
    for outcome in receipt_execution_outcomes {
        let events = extract_events(outcome);
        for event in events {
            res.extend(
                compose_rosetta_nep141_events(&event, outcome, block_header, view_client_addr)
                    .await?,
            );
        }
    }
    Ok(res)
}

async fn compose_rosetta_nep141_events(
    events: &crate::models::Nep141Event,
    outcome: &ExecutionOutcomeWithIdView,
    block_header: &near_primitives::views::BlockHeaderView,
    view_client_addr: &Addr<near_client::ViewClientActor>,
) -> crate::errors::Result<Vec<FungibleTokenEvent>> {
    let mut ft_events = Vec::new();
    match &events.event_kind {
        crate::models::Nep141EventKind::FtTransfer(transfer_events) => {
            for transfer_event in transfer_events {
                let base = get_base(Event::Nep141, outcome, block_header)?;
                // let ft_metadata = get_fungible_token_metadata(
                //     view_client_addr,
                //     block_header,
                //     &base.contract_account_id.address.to_string(),
                // )
                //.await?;
                let custom = crate::models::FtEvent {
                    affected_id: AccountIdentifier::from_str(&transfer_event.old_owner_id)?,
                    involved_id: Some(AccountIdentifier::from_str(&transfer_event.new_owner_id)?),
                    delta: -transfer_event.amount.parse::<i64>()?,
                    cause: "TRANSFER".to_string(),
                    memo: transfer_event.memo.as_ref().map(|s| s.escape_default().to_string()),
                    symbol: "near".to_string(),
                    decimals: 1,
                };
                ft_events.push(build_event(base, custom).await?);

                let base = get_base(Event::Nep141, outcome, block_header)?;
                let custom = crate::models::FtEvent {
                    affected_id: AccountIdentifier::from_str(&transfer_event.new_owner_id)?,
                    involved_id: Some(AccountIdentifier::from_str(&transfer_event.old_owner_id)?),
                    delta: transfer_event.amount.parse::<i64>()?,
                    cause: "TRANSFER".to_string(),
                    memo: transfer_event.memo.as_ref().map(|s| s.escape_default().to_string()),
                    symbol: "near".to_string(),
                    decimals: 1,
                };
                ft_events.push(build_event(base, custom).await?);
            }
        }
    }
    Ok(ft_events)
}

// async fn get_fungible_token_metadata(
//     view_client_addr: &actix::Addr<near_client::ViewClientActor>,
//     block_header: &near_primitives::views::BlockHeaderView,
//     contract_address: &String,
// ) -> crate::errors::Result<crate::models::FTMetadataResponse> {
//     let block_reference =
//         near_primitives::types::BlockReference::BlockId(BlockId::Hash(block_header.hash));
//     let request = near_primitives::views::QueryRequest::CallFunction {
//         account_id: near_account_id::AccountId::from_str(contract_address)?,
//         method_name: "ft_metadata".to_string(),
//         args: vec![].into(),
//     };
//     let query_response = view_client_addr
//         .send(near_client::Query { block_reference, request }.with_span_context())
//         .await?
//         .map_err(|e| crate::errors::ErrorKind::InternalInvariantError(e.to_string()))?;
//     let call_result = if let near_primitives::views::QueryResponseKind::CallResult(result) =
//         query_response.kind
//     {
//         result
//     } else {
//         return Err(crate::errors::ErrorKind::InternalInvariantError(format!(
//             "Couldn't retrieve metadata from contract address",
//         )));
//         //todo!()
//     };
//     let serde_call_result: crate::models::FTMetadataResponse =
//         serde_json::from_slice(&call_result.result).unwrap();
//     Ok(serde_call_result)
// }

pub(crate) fn extract_events(
    execution_outcome: &ExecutionOutcomeWithIdView,
) -> Vec<crate::models::Nep141Event> {
    let prefix = "EVENT_JSON:";
    execution_outcome
        .outcome
        .logs
        .iter()
        .filter_map(|untrimmed_log| {
            let log = untrimmed_log.trim();
            if !log.starts_with(prefix) {
                return None;
            }

            match serde_json::from_str::<'_, crate::models::Nep141Event>(log[prefix.len()..].trim())
            {
                Ok(result) => Some(result),
                Err(_err) => None,
            }
        })
        .collect()
}
pub(crate) fn get_base(
    event_type: Event,
    outcome: &ExecutionOutcomeWithIdView,
    block_header: &near_primitives::views::BlockHeaderView,
) -> crate::errors::Result<crate::models::EventBase> {
    Ok(crate::models::EventBase {
        standard: get_standard(&event_type),
        receipt_id: outcome.id,
        block_height: block_header.height,
        block_timestamp: block_header.timestamp,
        contract_account_id: outcome.outcome.executor_id.clone().into(),
        status: outcome.outcome.status.clone(),
    })
}

pub(crate) enum Event {
    Nep141,
}
fn get_standard(event_type: &Event) -> String {
    match event_type {
        Event::Nep141 => FT,
    }
    .to_string()
}
pub const FT: &str = "FT_NEP141";

async fn build_event(
    base: crate::models::EventBase,
    custom: crate::models::FtEvent,
) -> crate::errors::Result<FungibleTokenEvent> {
    Ok(FungibleTokenEvent {
        standard: base.standard,
        receipt_id: base.receipt_id,
        block_height: base.block_height,
        block_timestamp: base.block_timestamp,
        contract_account_id: base.contract_account_id.address.to_string(),
        symbol: custom.symbol,
        decimals: custom.decimals,
        affected_account_id: custom.affected_id.address.to_string(),
        involved_account_id: custom.involved_id.map(|id| id.address.to_string()),
        delta_amount: custom.delta,
        cause: custom.cause,
        status: get_status(&base.status),
        event_memo: custom.memo,
    })
}

fn get_status(status: &near_primitives::views::ExecutionStatusView) -> String {
    match status {
        near_primitives::views::ExecutionStatusView::Unknown => "UNKNOWN",
        near_primitives::views::ExecutionStatusView::Failure(_) => "FAILURE",
        near_primitives::views::ExecutionStatusView::SuccessValue(_) => "SUCCESS",
        near_primitives::views::ExecutionStatusView::SuccessReceiptId(_) => "SUCCESS",
    }
    .to_string()
}
