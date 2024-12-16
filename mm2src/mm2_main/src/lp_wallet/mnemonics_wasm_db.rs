use crate::lp_wallet::WalletsContext;
use async_trait::async_trait;
use crypto::EncryptedData;
use mm2_core::mm_ctx::MmArc;
use mm2_core::DbNamespaceId;
use mm2_db::indexed_db::{DbIdentifier, DbInstance, DbTransactionError, DbUpgrader, IndexedDb, IndexedDbBuilder,
                         InitDbError, InitDbResult, OnUpgradeError, OnUpgradeResult, TableSignature};
use mm2_err_handle::prelude::*;
use std::collections::HashMap;
use std::ops::Deref;

const DB_VERSION: u32 = 1;

type WalletsDBResult<T> = Result<T, MmError<WalletsDBError>>;

#[derive(Debug, Deserialize, Display, Serialize)]
pub enum WalletsDBError {
    #[display(fmt = "Error deserializing '{}': {}", field, error)]
    DeserializationError {
        field: String,
        error: String,
    },
    #[display(fmt = "Error serializing '{}': {}", field, error)]
    SerializationError {
        field: String,
        error: String,
    },
    Internal(String),
}

impl From<InitDbError> for WalletsDBError {
    fn from(e: InitDbError) -> Self { WalletsDBError::Internal(e.to_string()) }
}

impl From<DbTransactionError> for WalletsDBError {
    fn from(e: DbTransactionError) -> Self { WalletsDBError::Internal(e.to_string()) }
}

#[derive(Debug, Deserialize, Serialize)]
struct MnemonicsTable {
    wallet_name: String,
    encrypted_mnemonic: String,
}

pub struct WalletsDb {
    inner: IndexedDb,
}

#[async_trait]
impl DbInstance for WalletsDb {
    const DB_NAME: &'static str = "wallets";

    async fn init(db_id: DbIdentifier) -> InitDbResult<Self> {
        let inner = IndexedDbBuilder::new(db_id)
            .with_version(DB_VERSION)
            .with_table::<MnemonicsTable>()
            .build()
            .await?;
        Ok(WalletsDb { inner })
    }
}

impl Deref for WalletsDb {
    type Target = IndexedDb;

    fn deref(&self) -> &Self::Target { &self.inner }
}

impl TableSignature for MnemonicsTable {
    const TABLE_NAME: &'static str = "mnemonics";

    fn on_upgrade_needed(upgrader: &DbUpgrader, mut old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        while old_version < new_version {
            match old_version {
                0 => {
                    let table = upgrader.create_table(Self::TABLE_NAME)?;
                    table.create_index("wallet_name", true)?;
                },
                // handle new versions here if needed
                unsupported_version => {
                    return MmError::err(OnUpgradeError::UnsupportedVersion {
                        unsupported_version,
                        old_version,
                        new_version,
                    })
                },
            }

            old_version += 1;
        }
        Ok(())
    }
}

pub(super) async fn save_encrypted_passphrase(
    ctx: &MmArc,
    wallet_name: &str,
    encrypted_passphrase_data: &EncryptedData,
) -> WalletsDBResult<()> {
    let wallets_ctx = WalletsContext::from_ctx(ctx).map_to_mm(WalletsDBError::Internal)?;

    let db = wallets_ctx.wallets_db().await?;
    let transaction = db.transaction().await?;
    let table = transaction.table::<MnemonicsTable>().await?;

    let mnemonics_table_item = MnemonicsTable {
        wallet_name: wallet_name.to_string(),
        encrypted_mnemonic: serde_json::to_string(encrypted_passphrase_data).map_err(|e| {
            WalletsDBError::SerializationError {
                field: "encrypted_mnemonic".to_string(),
                error: e.to_string(),
            }
        })?,
    };
    table.add_item(&mnemonics_table_item).await?;

    Ok(())
}

pub(super) async fn read_encrypted_passphrase_if_available(ctx: &MmArc) -> WalletsDBResult<Option<EncryptedData>> {
    let wallets_ctx = WalletsContext::from_ctx(ctx).map_to_mm(WalletsDBError::Internal)?;

    let db = wallets_ctx.wallets_db().await?;
    let transaction = db.transaction().await?;
    let table = transaction.table::<MnemonicsTable>().await?;

    let wallet_name = ctx
        .wallet_name
        .get()
        .ok_or(WalletsDBError::Internal(
            "`wallet_name` not initialized yet!".to_string(),
        ))?
        .clone()
        .ok_or_else(|| WalletsDBError::Internal("`wallet_name` can't be None!".to_string()))?;
    table
        .get_item_by_unique_index("wallet_name", wallet_name)
        .await?
        .map(|(_item_id, wallet_table_item)| {
            serde_json::from_str(&wallet_table_item.encrypted_mnemonic).map_to_mm(|e| {
                WalletsDBError::DeserializationError {
                    field: "encrypted_mnemonic".to_string(),
                    error: e.to_string(),
                }
            })
        })
        .transpose()
}

pub(super) async fn read_all_wallet_names(ctx: &MmArc) -> WalletsDBResult<impl Iterator<Item = String>> {
    let wallets_ctx = WalletsContext::from_ctx(ctx).map_to_mm(WalletsDBError::Internal)?;

    let db = wallets_ctx.wallets_db().await?;
    let transaction = db.transaction().await?;
    let table = transaction.table::<MnemonicsTable>().await?;

    let all_items = table.get_all_items().await?;
    let wallet_names = all_items.into_iter().map(|(_, item)| item.wallet_name);

    Ok(wallet_names)
}
