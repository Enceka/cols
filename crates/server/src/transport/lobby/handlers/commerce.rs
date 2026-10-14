use crate::backend::commerce::marketplace::Payment;
use crate::backend::commerce::service::CommerceService;
use crate::backend::identity::session::Session;
use crate::backend::inventory::InventoryService;
use crate::contracts::lobby::TaskResult;
use anyhow::Result;
use codol_protocol::lobby::task::{self, Request, Writer};
pub(crate) async fn handle(
    commerce: &CommerceService,
    inventory: &InventoryService,
    request: &Request<'_>,
    session: &Session,
) -> Result<TaskResult> {
    match (request.service, request.task) {
        (80, 70 | 107) => {
            let mut reader = task::Reader::new(request.arguments);
            let context = reader.string(16)?;
            let page = reader.u32()?;
            let page_size = reader.u32()?;
            if request.task == 70 {
                let _show_all = reader.boolean()?;
                let sku_filters = reader.u32()?;
                if sku_filters != 0 {
                    return Ok(TaskResult::failure(
                        "unsupported_catalogue_filter",
                        task::TASK_FAILED,
                    ));
                }
                let type_filters = reader.u32()?;
                if type_filters != 0 {
                    return Ok(TaskResult::failure(
                        "unsupported_catalogue_filter",
                        task::TASK_FAILED,
                    ));
                }
            }
            let _client_version = reader.string(32)?;
            reader.finish()?;
            if !context.is_empty() || page != 1 || !(1..=2000).contains(&page_size) {
                return Ok(TaskResult::failure(
                    "unsupported_catalogue_query",
                    task::TASK_FAILED,
                ));
            }
            // VA 0x4a6150 stops pagination when resultCount < pageSize,
            // including an empty first page. The lab has no saleable SKUs;
            // owned inventory and starter grants remain separate work.
            Ok(TaskResult::success(
                if request.task == 70 {
                    "local_empty_catalogue"
                } else {
                    "local_empty_catalogue_links"
                },
                vec![],
            ))
        }
        (80, 230) => {
            let mut reader = task::Reader::new(request.arguments);
            let context = reader.string(16)?;
            reader.finish()?;
            if !context.is_empty() {
                return Ok(TaskResult::failure(
                    "unsupported_marketplace_namespace",
                    task::TASK_FAILED,
                ));
            }
            let mut items = inventory.items(session).await?;
            let wallet = commerce.wallet(session).await?;
            if let Some(w) = &wallet {
                for grant in &w.purchases {
                    if !items.contains(&grant.item_id) {
                        items.push(grant.item_id);
                    }
                }
            }
            let balances = wallet
                .as_ref()
                // VA 0x4a5680 copies ONLY entry 0 to the marketplace wallet.
                // It must be GP; the separate 93/1 result supplies CP.
                .map(|w| vec![(Payment::Gp.wire_currency(), w.gp)])
                .unwrap_or_default();
            Ok(TaskResult::success(
                if wallet.is_some() {
                    "local_player_wallet_inventory"
                } else if items.is_empty() {
                    "local_empty_player_data"
                } else {
                    "local_player_inventory"
                },
                vec![task::player_data_with_balances(
                    session.user_id,
                    &items,
                    &balances,
                )?],
            ))
        }
        (80, 173) => {
            let mut reader = task::Reader::new(request.arguments);
            let context = reader.string(16)?;
            let maximum = reader.u16()?;
            reader.finish()?;
            if !context.is_empty() || !(1..=700).contains(&maximum) {
                return Ok(TaskResult::failure(
                    "unsupported_coupon_query",
                    task::TASK_FAILED,
                ));
            }
            // VA 0xa46fb0 binds an array; VA 0x4a3470 consumes its count.
            // This local account has no issued coupons/mail attachments.
            Ok(TaskResult::success("empty_coupon_inventory", vec![]))
        }
        (93, 1) | (80, 130) => {
            let mut reader = task::Reader::new(request.arguments);
            let maximum = reader.u32()?;
            reader.finish()?;
            if maximum != 1 {
                return Ok(TaskResult::failure(
                    "unsupported_balance_query",
                    task::TASK_FAILED,
                ));
            }
            // VA 0xa4c4a0 reads UChar + UInt32. Payment route and UI wire
            // currency are distinct; the target UI uses GP=1 and CP=2.
            let cp = request.service == 93;
            let amount = commerce
                .wallet(session)
                .await?
                .map_or(0, |w| if cp { w.cp } else { w.gp });
            let mut record = Writer::default();
            record.u8(if cp { Payment::Cp } else { Payment::Gp }.wire_currency());
            record.u32(amount);
            Ok(TaskResult::success(
                if cp {
                    "local_cp_balance"
                } else {
                    "local_gp_balance"
                },
                vec![record.0],
            ))
        }
        (80, 203) => {
            let mut r = task::Reader::new(request.arguments);
            let context = r.string(16)?;
            let maximum = r.u16()?;
            r.finish()?;
            if !context.is_empty() || !(1..=20000).contains(&maximum) {
                return Ok(TaskResult::failure(
                    "unsupported_inventory_query",
                    task::TASK_FAILED,
                ));
            }
            let mut items = inventory.items(session).await?;
            if let Some(wallet) = commerce.wallet(session).await? {
                for p in wallet.purchases {
                    if !items.contains(&p.item_id) {
                        items.push(p.item_id);
                    }
                }
            }
            let mut records = task::inventory_records(session.user_id, &items)?;
            records.truncate(usize::from(maximum));
            Ok(TaskResult::success("local_inventory_refresh", records))
        }
        (80, 154) | (93, 131) => {
            let sku =
                codol_protocol::lobby::purchase::single_sku(request.service, request.arguments)?;
            let payment = if request.service == 93 {
                Payment::Cp
            } else {
                Payment::Gp
            };
            match commerce.purchase(session, inventory, sku, payment).await? {
                Ok(()) => Ok(TaskResult::success(
                    "local_permanent_item_purchased",
                    vec![],
                )),
                Err(label) => Ok(TaskResult::failure(label, task::TASK_FAILED)),
            }
        }
        _ => Ok(TaskResult::failure("unsupported_task", task::TASK_FAILED)),
    }
}
