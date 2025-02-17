use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::{MmError, MmResult};
use std::str::FromStr;

pub(crate) mod nft_errors;
pub(crate) mod nft_structs;
#[cfg(any(test, target_arch = "wasm32"))] mod nft_tests;

use crate::WithdrawError;
use nft_errors::{GetInfoFromUriError, GetNftInfoError};
use nft_structs::{ConvertChain, Nft, NftList, NftListReq, NftMetadataReq, NftTransferHistory,
                  NftTransferHistoryWrapper, NftTransfersReq, NftWrapper, NftsTransferHistoryList,
                  TransactionNftDetails, WithdrawNftReq};

use crate::eth::{get_eth_address, withdraw_erc1155, withdraw_erc721};
use crate::nft::nft_structs::{TransferStatus, UriMeta, WithdrawNftType};
use common::APPLICATION_JSON;
use ethereum_types::Address;
use http::header::ACCEPT;
use mm2_err_handle::map_to_mm::MapToMmResult;
use mm2_number::BigDecimal;
use serde_json::Value as Json;

const MORALIS_API_ENDPOINT: &str = "api/v2";
/// query parameters for moralis request: The format of the token ID
const MORALIS_FORMAT_QUERY_NAME: &str = "format";
const MORALIS_FORMAT_QUERY_VALUE: &str = "decimal";
/// query parameters for moralis request: The transfer direction
const MORALIS_DIRECTION_QUERY_NAME: &str = "direction";
const MORALIS_DIRECTION_QUERY_VALUE: &str = "both";

pub type WithdrawNftResult = Result<TransactionNftDetails, MmError<WithdrawError>>;

/// `get_nft_list` function returns list of NFTs on requested chains owned by user.
pub async fn get_nft_list(ctx: MmArc, req: NftListReq) -> MmResult<NftList, GetNftInfoError> {
    let mut res_list = Vec::new();
    for chain in req.chains {
        let my_address = get_eth_address(&ctx, &chain.to_ticker()).await?;

        let mut uri_without_cursor = req.url.clone();
        uri_without_cursor.set_path(MORALIS_API_ENDPOINT);
        uri_without_cursor
            .path_segments_mut()
            .map_to_mm(|_| GetNftInfoError::Internal("Invalid URI".to_string()))?
            .push(&my_address.wallet_address)
            .push("nft");
        uri_without_cursor
            .query_pairs_mut()
            .append_pair("chain", &chain.to_string())
            .append_pair(MORALIS_FORMAT_QUERY_NAME, MORALIS_FORMAT_QUERY_VALUE);
        drop_mutability!(uri_without_cursor);

        // The cursor returned in the previous response (used for getting the next page).
        let mut cursor = String::new();
        loop {
            let uri = format!("{}{}", uri_without_cursor, cursor);
            let response = send_request_to_uri(uri.as_str()).await?;
            if let Some(nfts_list) = response["result"].as_array() {
                for nft_json in nfts_list {
                    let nft_wrapper: NftWrapper = serde_json::from_str(&nft_json.to_string())?;
                    let uri_meta = try_get_uri_meta(&nft_wrapper.token_uri).await?;
                    let nft = Nft {
                        chain,
                        token_address: nft_wrapper.token_address,
                        token_id: nft_wrapper.token_id.0,
                        amount: nft_wrapper.amount.0,
                        owner_of: nft_wrapper.owner_of,
                        token_hash: nft_wrapper.token_hash,
                        block_number_minted: *nft_wrapper.block_number_minted,
                        block_number: *nft_wrapper.block_number,
                        contract_type: nft_wrapper.contract_type.map(|v| v.0),
                        collection_name: nft_wrapper.name,
                        symbol: nft_wrapper.symbol,
                        token_uri: nft_wrapper.token_uri,
                        metadata: nft_wrapper.metadata,
                        last_token_uri_sync: nft_wrapper.last_token_uri_sync,
                        last_metadata_sync: nft_wrapper.last_metadata_sync,
                        minter_address: nft_wrapper.minter_address,
                        possible_spam: nft_wrapper.possible_spam,
                        uri_meta,
                    };
                    // collect NFTs from the page
                    res_list.push(nft);
                }
                // if cursor is not null, there are other NFTs on next page,
                // and we need to send new request with cursor to get info from the next page.
                if let Some(cursor_res) = response["cursor"].as_str() {
                    cursor = format!("{}{}", "&cursor=", cursor_res);
                    continue;
                } else {
                    break;
                }
            }
        }
    }
    drop_mutability!(res_list);
    let nft_list = NftList { nfts: res_list };
    Ok(nft_list)
}

/// `get_nft_metadata` function returns info of one specific NFT.
/// Current implementation sends request to Moralis.
/// Later, after adding caching, metadata lookup can be performed using previously obtained NFTs info without
/// sending new moralis request. The moralis request can be sent as a fallback, if the data was not found in the cache.
///
/// **Caution:** ERC-1155 token can have a total supply more than 1, which means there could be several owners
/// of the same token. `get_nft_metadata` returns NFTs info with the most recent owner.
/// **Dont** use this function to get specific info about owner address, amount etc, you will get info not related to my_address.
pub async fn get_nft_metadata(_ctx: MmArc, req: NftMetadataReq) -> MmResult<Nft, GetNftInfoError> {
    let mut uri = req.url;
    uri.set_path(MORALIS_API_ENDPOINT);
    uri.path_segments_mut()
        .map_to_mm(|_| GetNftInfoError::Internal("Invalid URI".to_string()))?
        .push("nft")
        .push(&format!("{:#02x}", &req.token_address))
        .push(&req.token_id.to_string());
    uri.query_pairs_mut()
        .append_pair("chain", &req.chain.to_string())
        .append_pair(MORALIS_FORMAT_QUERY_NAME, MORALIS_FORMAT_QUERY_VALUE);
    drop_mutability!(uri);

    let response = send_request_to_uri(uri.as_str()).await?;
    let nft_wrapper: NftWrapper = serde_json::from_str(&response.to_string())?;
    let uri_meta = try_get_uri_meta(&nft_wrapper.token_uri).await?;
    let nft_metadata = Nft {
        chain: req.chain,
        token_address: nft_wrapper.token_address,
        token_id: nft_wrapper.token_id.0,
        amount: nft_wrapper.amount.0,
        owner_of: nft_wrapper.owner_of,
        token_hash: nft_wrapper.token_hash,
        block_number_minted: *nft_wrapper.block_number_minted,
        block_number: *nft_wrapper.block_number,
        contract_type: nft_wrapper.contract_type.map(|v| v.0),
        collection_name: nft_wrapper.name,
        symbol: nft_wrapper.symbol,
        token_uri: nft_wrapper.token_uri,
        metadata: nft_wrapper.metadata,
        last_token_uri_sync: nft_wrapper.last_token_uri_sync,
        last_metadata_sync: nft_wrapper.last_metadata_sync,
        minter_address: nft_wrapper.minter_address,
        possible_spam: nft_wrapper.possible_spam,
        uri_meta,
    };
    Ok(nft_metadata)
}

/// `get_nft_transfers` function returns a transfer history of NFTs on requested chains owned by user.
/// Currently doesnt support filters.
pub async fn get_nft_transfers(ctx: MmArc, req: NftTransfersReq) -> MmResult<NftsTransferHistoryList, GetNftInfoError> {
    let mut res_list = Vec::new();

    for chain in req.chains {
        let my_address = get_eth_address(&ctx, &chain.to_ticker()).await?;

        let mut uri_without_cursor = req.url.clone();
        uri_without_cursor.set_path(MORALIS_API_ENDPOINT);
        uri_without_cursor
            .path_segments_mut()
            .map_to_mm(|_| GetNftInfoError::Internal("Invalid URI".to_string()))?
            .push(&my_address.wallet_address)
            .push("nft")
            .push("transfers");
        uri_without_cursor
            .query_pairs_mut()
            .append_pair("chain", &chain.to_string())
            .append_pair(MORALIS_FORMAT_QUERY_NAME, MORALIS_FORMAT_QUERY_VALUE)
            .append_pair(MORALIS_DIRECTION_QUERY_NAME, MORALIS_DIRECTION_QUERY_VALUE);
        drop_mutability!(uri_without_cursor);

        // The cursor returned in the previous response (used for getting the next page).
        let mut cursor = String::new();
        let wallet_address = my_address.wallet_address;
        loop {
            let uri = format!("{}{}", uri_without_cursor, cursor);
            let response = send_request_to_uri(uri.as_str()).await?;
            if let Some(transfer_list) = response["result"].as_array() {
                for transfer in transfer_list {
                    let transfer_wrapper: NftTransferHistoryWrapper = serde_json::from_str(&transfer.to_string())?;
                    let status = get_tx_status(&wallet_address, &transfer_wrapper.to_address);
                    let req = NftMetadataReq {
                        token_address: Address::from_str(&transfer_wrapper.token_address)
                            .map_to_mm(|e| GetNftInfoError::AddressError(e.to_string()))?,
                        token_id: transfer_wrapper.token_id.clone(),
                        chain,
                        url: req.url.clone(),
                    };
                    let nft_meta = get_nft_metadata(ctx.clone(), req).await?;
                    let transfer_history = NftTransferHistory {
                        chain,
                        block_number: *transfer_wrapper.block_number,
                        block_timestamp: transfer_wrapper.block_timestamp,
                        block_hash: transfer_wrapper.block_hash,
                        transaction_hash: transfer_wrapper.transaction_hash,
                        transaction_index: transfer_wrapper.transaction_index,
                        log_index: transfer_wrapper.log_index,
                        value: transfer_wrapper.value.0,
                        contract_type: transfer_wrapper.contract_type.0,
                        transaction_type: transfer_wrapper.transaction_type,
                        token_address: transfer_wrapper.token_address,
                        token_id: transfer_wrapper.token_id.0,
                        collection_name: nft_meta.collection_name,
                        image: nft_meta.uri_meta.image,
                        token_name: nft_meta.uri_meta.token_name,
                        from_address: transfer_wrapper.from_address,
                        to_address: transfer_wrapper.to_address,
                        status,
                        amount: transfer_wrapper.amount.0,
                        verified: transfer_wrapper.verified,
                        operator: transfer_wrapper.operator,
                        possible_spam: transfer_wrapper.possible_spam,
                    };
                    // collect NFTs transfers from the page
                    res_list.push(transfer_history);
                }
                // if the cursor is not null, there are other NFTs transfers on next page,
                // and we need to send new request with cursor to get info from the next page.
                if let Some(cursor_res) = response["cursor"].as_str() {
                    cursor = format!("{}{}", "&cursor=", cursor_res);
                    continue;
                } else {
                    break;
                }
            }
        }
    }
    drop_mutability!(res_list);
    let transfer_history_list = NftsTransferHistoryList {
        transfer_history: res_list,
    };
    Ok(transfer_history_list)
}

/// `withdraw_nft` function generates, signs and returns a transaction that transfers NFT
/// from my address to recipient's address.
/// This method generates a raw transaction which should then be broadcast using `send_raw_transaction`.
pub async fn withdraw_nft(ctx: MmArc, req: WithdrawNftReq) -> WithdrawNftResult {
    match req.withdraw_type {
        WithdrawNftType::WithdrawErc1155(erc1155_withdraw) => withdraw_erc1155(ctx, erc1155_withdraw, req.url).await,
        WithdrawNftType::WithdrawErc721(erc721_withdraw) => withdraw_erc721(ctx, erc721_withdraw).await,
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn send_request_to_uri(uri: &str) -> MmResult<Json, GetInfoFromUriError> {
    use http::header::HeaderValue;
    use mm2_net::transport::slurp_req_body;

    let request = http::Request::builder()
        .method("GET")
        .uri(uri)
        .header(ACCEPT, HeaderValue::from_static(APPLICATION_JSON))
        .body(hyper::Body::from(""))?;

    let (status, _header, body) = slurp_req_body(request).await?;
    if !status.is_success() {
        return Err(MmError::new(GetInfoFromUriError::Transport(format!(
            "Response !200 from {}: {}, {}",
            uri, status, body
        ))));
    }
    Ok(body)
}

#[cfg(target_arch = "wasm32")]
async fn send_request_to_uri(uri: &str) -> MmResult<Json, GetInfoFromUriError> {
    use mm2_net::wasm_http::FetchRequest;

    macro_rules! try_or {
        ($exp:expr, $errtype:ident) => {
            match $exp {
                Ok(x) => x,
                Err(e) => return Err(MmError::new(GetInfoFromUriError::$errtype(ERRL!("{:?}", e)))),
            }
        };
    }

    let result = FetchRequest::get(uri)
        .header(ACCEPT.as_str(), APPLICATION_JSON)
        .request_str()
        .await;
    let (status_code, response_str) = try_or!(result, Transport);
    if !status_code.is_success() {
        return Err(MmError::new(GetInfoFromUriError::Transport(ERRL!(
            "!200: {}, {}",
            status_code,
            response_str
        ))));
    }

    let response: Json = try_or!(serde_json::from_str(&response_str), InvalidResponse);
    Ok(response)
}

/// This function uses `get_nft_list` method to get the correct info about amount of specific NFT owned by my_address.
pub(crate) async fn find_wallet_amount(
    ctx: MmArc,
    nft_list: NftListReq,
    token_address_req: String,
    token_id_req: BigDecimal,
) -> MmResult<BigDecimal, GetNftInfoError> {
    let nft_list = get_nft_list(ctx, nft_list).await?.nfts;
    let nft = nft_list
        .into_iter()
        .find(|nft| nft.token_address == token_address_req && nft.token_id == token_id_req)
        .ok_or_else(|| GetNftInfoError::TokenNotFoundInWallet {
            token_address: token_address_req,
            token_id: token_id_req.to_string(),
        })?;
    Ok(nft.amount)
}

async fn try_get_uri_meta(token_uri: &Option<String>) -> MmResult<UriMeta, GetNftInfoError> {
    match token_uri {
        Some(token_uri) => {
            if let Ok(response_meta) = send_request_to_uri(token_uri).await {
                let uri_meta_res: UriMeta = serde_json::from_str(&response_meta.to_string())?;
                Ok(uri_meta_res)
            } else {
                Ok(UriMeta::default())
            }
        },
        None => Ok(UriMeta::default()),
    }
}

fn get_tx_status(my_wallet: &str, to_address: &str) -> TransferStatus {
    // if my_wallet == from_address && my_wallet == to_address it is incoming tx, so we can check just to_address.
    if my_wallet.to_lowercase() == to_address.to_lowercase() {
        TransferStatus::Receive
    } else {
        TransferStatus::Send
    }
}
