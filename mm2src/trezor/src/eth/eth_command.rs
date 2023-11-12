use crate::proto::messages_ethereum as proto_ethereum;
use crate::result_handler::ResultHandler;
use crate::{serialize_derivation_path, OperationFailure, TrezorError, TrezorResponse, TrezorResult, TrezorSession};
use bip32::DerivationPath;
use ethcore_transaction::{Action, Transaction as UnSignedEthTx, UnverifiedTransaction as UnverifiedEthTx};
use ethkey::Signature;
use hw_common::primitives::XPub;
use mm2_err_handle::prelude::MmError;
use primitive_types::H256;

impl<'a> TrezorSession<'a> {
    pub async fn get_eth_address(&mut self, derivation_path: DerivationPath) -> TrezorResult<Option<String>> {
        let req = proto_ethereum::EthereumGetAddress {
            address_n: derivation_path.iter().map(|child| child.0).collect(),
            show_display: None,
            encoded_network: None,
            chunkify: None,
        };
        //let mut tx_request = self.send_get_eth_address_req(req).await?.ack_all().await?;
        let result_handler = ResultHandler::<proto_ethereum::EthereumAddress>::new(Ok);
        let result = self.call(req, result_handler).await?.ack_all().await?;
        Ok(result.address)
    }

    pub async fn get_eth_public_key<'b>(
        &'b mut self,
        derivation_path: DerivationPath,
        show_display: bool,
    ) -> TrezorResult<TrezorResponse<'a, 'b, XPub>> {
        let req = proto_ethereum::EthereumGetPublicKey {
            address_n: serialize_derivation_path(&derivation_path),
            show_display: Some(show_display),
        };
        let result_handler = ResultHandler::new(|m: proto_ethereum::EthereumPublicKey| Ok(m.xpub));
        self.call(req, result_handler).await
    }

    pub async fn sign_eth_tx(
        &mut self,
        derivation_path: DerivationPath,
        unsigned_tx: &UnSignedEthTx,
        chain_id: u64,
    ) -> TrezorResult<UnverifiedEthTx> {
        let mut data: Vec<u8> = vec![];
        let req = to_sign_eth_message(unsigned_tx, &derivation_path, chain_id, &mut data);
        let mut tx_request = self.send_sign_eth_tx(req).await?.ack_all().await?;

        while let Some(data_length) = tx_request.data_length {
            if data_length > 0 {
                println!("data_length={}", data_length);
                let req = proto_ethereum::EthereumTxAck {
                    data_chunk: data.splice(..std::cmp::min(1024, data.len()), []).collect(),
                };
                //ack.set_data_chunk(data.splice(..std::cmp::min(1024, data.len()), []).collect());

                //resp = self.call(ack, Box::new(|_, m: proto_ethereum::EthereumTxRequest| Ok(m)))?.ok()?;
                tx_request = self.send_eth_tx_ack(req).await?.ack_all().await?;
            } else {
                break;
            }
        }

        let sig = extract_eth_signature(&tx_request, chain_id)?;
        Ok(unsigned_tx.clone().with_signature(sig, Some(chain_id)))
    }

    async fn send_sign_eth_tx<'b>(
        &'b mut self,
        req: proto_ethereum::EthereumSignTx,
    ) -> TrezorResult<TrezorResponse<'a, 'b, proto_ethereum::EthereumTxRequest>> {
        let result_handler = ResultHandler::<proto_ethereum::EthereumTxRequest>::new(Ok);
        self.call(req, result_handler).await
    }

    async fn send_eth_tx_ack<'b>(
        &'b mut self,
        req: proto_ethereum::EthereumTxAck,
    ) -> TrezorResult<TrezorResponse<'a, 'b, proto_ethereum::EthereumTxRequest>> {
        let result_handler = ResultHandler::<proto_ethereum::EthereumTxRequest>::new(Ok);
        self.call(req, result_handler).await
    }
}

/// TODO: maybe there is a more standard way
fn my_trim_u8(arr: &[u8]) -> Vec<u8> {
    let mut z = 0;
    for i in arr {
        if i == &0 {
            z += 1;
        }
    }
    arr[z..].to_vec()
}

fn to_sign_eth_message(
    unsigned_tx: &UnSignedEthTx,
    derivation_path: &DerivationPath,
    chain_id: u64,
    data: &mut Vec<u8>,
) -> proto_ethereum::EthereumSignTx {
    let mut nonce: [u8; 32] = [0; 32];
    let mut gas_price: [u8; 32] = [0; 32];
    let mut gas_limit: [u8; 32] = [0; 32];
    let mut value: [u8; 32] = [0; 32];

    unsigned_tx.nonce.to_big_endian(&mut nonce);
    unsigned_tx.gas_price.to_big_endian(&mut gas_price);
    unsigned_tx.gas.to_big_endian(&mut gas_limit);
    unsigned_tx.value.to_big_endian(&mut value);

    *data = unsigned_tx.data.clone();
    let addr_hex = if let Action::Call(addr) = unsigned_tx.action {
        Some(format!("{:X}", addr))
    } else {
        None
    };
    proto_ethereum::EthereumSignTx {
        address_n: serialize_derivation_path(derivation_path), // derivation_path.iter().map(|child| child.0).collect(),
        nonce: Some(my_trim_u8(&nonce)),
        gas_price: my_trim_u8(&gas_price),
        gas_limit: my_trim_u8(&gas_limit),
        to: addr_hex,
        value: Some(my_trim_u8(&value)),
        data_initial_chunk: Some(data.splice(..std::cmp::min(1024, data.len()), []).collect()),
        data_length: if data.is_empty() { None } else { Some(data.len() as u32) },
        chain_id,
        tx_type: None,
        definitions: None,
        chunkify: if data.is_empty() { None } else { Some(true) },
    }
}

fn extract_eth_signature(tx_request: &proto_ethereum::EthereumTxRequest, chain_id: u64) -> TrezorResult<Signature> {
    match (
        tx_request.signature_r.as_ref(),
        tx_request.signature_s.as_ref(),
        tx_request.signature_v,
    ) {
        (Some(r), Some(s), Some(v)) => {
            let v_fixed = if v <= 1 { v + 2 * (chain_id as u32) + 35 } else { v };
            Ok(Signature::from_rsv(
                &H256::from_slice(r.as_slice()),
                &H256::from_slice(s.as_slice()),
                v_fixed as u8,
            ))
        },
        (_, _, _) => Err(MmError::new(TrezorError::Failure(OperationFailure::InvalidSignature))),
    }
}
