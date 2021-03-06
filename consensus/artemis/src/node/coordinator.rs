use types::{BlockTrait, artemis::{Block, ProtocolMsg, Transaction}};
use types::WireReady;
use super::context::Context;
use std::sync::Arc;

/// Dispatch block is called by the view leader to create candidate blocks and send it to all the nodes
pub async fn do_new_block(txs: Vec<Arc<Transaction>>, cx:&mut Context)
{
    log::debug!("View leader dispatching a block");
    let mut new_block = Block::with_tx(txs);
    new_block.blk.header.prev = cx.last_seen_block.get_hash();
    new_block.blk.header.author = cx.myid();
    new_block.blk.header.height = cx.last_seen_block.get_height()+1;
    new_block.sig.origin = cx.myid();
    let mut new_block = new_block.init();
    new_block.sign(&cx.my_secret_key);
    
    // Send this new block to everyone
    let msg = Arc::new(ProtocolMsg::RawNewBlock(new_block.clone()));
    cx.multicast(msg.clone()).await;
    log::debug!("Broadcasting new blocks to all the nodes");
    
    // Process this new block
    on_receive_new_block_direct(cx, new_block).await;
}

/// `on_recv_new_block_direct` is called when we get a new block from the view co-ordinator (directly)
/// A Byzantine node may deliver out-of-order blocks; Discard a block that does not extend the block that it sent last
pub async fn on_receive_new_block_direct(cx:&mut Context, blk: Block) {
    log::debug!("Got a new block from the view leader: {:?}", blk);
    if cx.storage.is_delivered_by_hash(&blk.get_hash()) {
        return;
    }
    
    // Check if the block is delivered
    if !cx.storage.is_delivered_by_hash(&blk.blk.header.prev) {
        log::warn!("View leader sent out of order blocks");
        return;
    }
    // Check if the origin fields are correct
    if cx.view_leader != blk.get_author() || cx.view_leader != blk.sig.origin {
        log::warn!("Got an invalid block. Expected block from the view leader ({}), got a block from {} with sig from {}", cx.view_leader, blk.get_author(), blk.sig.origin);
        return;
    }
    // Check if this is signed correctly
    // Ignore checking signature if I signed it myself
    if cx.view_leader != cx.myid() && !blk.check_sig(cx.pub_key_map.get(&cx.view_leader).expect("Must have this node's pubkey")) {
        log::warn!("Got an invalid signature");
        return;
    }
    log::debug!("Successfully dealt with the view leader's block: {:?}", blk);
    // We have a valid signed and delivered block
    do_delivery(blk,cx);
    
}

pub fn do_delivery(blk: Block, cx:&mut Context) {
    // Add it to storage
    let b_hash = blk.get_hash();
    let b_rc = Arc::new(blk);
    cx.storage.add_delivered_block(b_rc.clone());
    // We have a new delivered block
    if cx.last_seen_block.get_height() < b_rc.get_height() {
        cx.last_seen_block = b_rc;
    }
    
    // If this was undelivered remove it
    cx.undelivered_blocks.remove(&b_hash);
    
    // Check if any vote gets delivered because this block got delivered
    if let Some(v) = cx.vote_waiting.remove(&b_hash) {
        cx.vote_ready.insert(v.round, v);
    }
    
    let mut b_hash = b_hash;
    // If some block was waiting for this block to be delivered
    while let Some(child) = cx.block_parent_waiting.remove(&b_hash) {
        // This block may trigger delivery of children
        if let Some(b) = cx.undelivered_blocks.remove(&child) {
            // We have a new delivered block
            let b_rc = Arc::new(b);
            cx.storage.add_delivered_block(b_rc.clone());
            if cx.last_seen_block.get_height() < b_rc.get_height() {
                cx.last_seen_block = b_rc;
            }
        }
        // Check if any vote gets delivered because this block got delivered
        if let Some(v) = cx.vote_waiting.remove(&child) {
            cx.vote_ready.insert(v.round, v);
        }
        // Repeat these steps with the block (child) that was waiting for this block
        b_hash = child;
    }
} 