use crate::block::{BlockFinality, ShardBlock};
use crate::data::{NodeStorage, ShardStorage};
use crate::tests::{
    find_block_by_hash, generate_block_with_seq_no, hexdump, mem_storage, rollback_to,
};
use std::io::Cursor;
use std::sync::Arc;
use ever_block::{
    BlkPrevInfo, Block, ExtBlkRef, GetRepresentationHash, ShardIdent, ShardStateUnsplit,
};

#[test]
fn test_serialize_shard_block_empty_with_default_signedblock() {
    let block = Block::default();

    let ss = ShardStateUnsplit::default();

    let sb = ShardBlock::with_block_and_state(block, Arc::new(ss));

    let data = sb.serialize().unwrap();

    hexdump(&data[..]);

    let sb_restored = ShardBlock::deserialize(&mut Cursor::new(data)).unwrap();
    assert_eq!(sb, sb_restored);
}

#[test]
fn test_serialize_shard_block_empty() {
    let block = Block::default();

    let ss = ShardStateUnsplit::default();

    let sb = ShardBlock::with_block_and_state(block, Arc::new(ss));

    let data = sb.serialize().unwrap();

    hexdump(&data[..]);

    let sb_restored = ShardBlock::deserialize(&mut Cursor::new(data)).unwrap();
    assert_eq!(sb, sb_restored);
}

#[test]
fn test_serialize_shard_block() {
    let shard_ident = ShardIdent::with_prefix_len(2, 0, 0xF0000000_00000000).unwrap();
    let prev_info = BlkPrevInfo::default();
    // todo: 5000
    let block = generate_block_with_seq_no(shard_ident.clone(), 1, prev_info, 500);
    let ss = ShardStateUnsplit::default();

    let sb = ShardBlock::with_block_and_state(block, Arc::new(ss));

    let data = sb.serialize().unwrap();

    let sb_restored = ShardBlock::deserialize(&mut Cursor::new(data)).unwrap();
    assert_eq!(sb, sb_restored);
}

#[test]
#[ignore]
fn test_block_finality() {
    // shard
    let shard_ident = ShardIdent::with_prefix_len(2, 0, 0xF0000000_00000000).unwrap();

    // validator key
    let mut prev_info = BlkPrevInfo::default();
    let mut blocks = vec![];
    let mut blocks_hashes = vec![];

    // generate some blocks
    for n in 0..12 {
        // todo: 5000
        let block = generate_block_with_seq_no(shard_ident.clone(), n + 1, prev_info, 500);
        let block_hash = block.hash().unwrap();
        prev_info = BlkPrevInfo::Block {
            prev: ExtBlkRef {
                end_lt: block.read_info().unwrap().end_lt(),
                seq_no: block.read_info().unwrap().seq_no(),
                root_hash: block_hash.clone(),
                file_hash: block_hash.clone(),
            },
        };
        blocks.push(block);
        blocks_hashes.push(block_hash);
    }

    let test_storage = mem_storage();
    let mut block_finality = BlockFinality::with_params(
        0,
        shard_ident.clone(),
        ShardStorage::new(test_storage.shard_storage(shard_ident.clone()).unwrap()),
        None,
    );

    // after 5 blocks set finaliti hash from 1
    for n in 0..10 {
        println!("round: {}", n);
        println!("block hash: {:?}", blocks_hashes[n].clone());
        let ss = ShardStateUnsplit::default();
        let fin_hash = if n >= 5 {
            Some(blocks_hashes[n - 5].clone())
        } else {
            None
        };
        println!("finality block hash: {:?}", fin_hash);

        block_finality
            .put_block_with_info(1, blocks[n].clone(), Arc::new(ss), Default::default())
            .unwrap();

        let expected_count = (if (5..=8).contains(&n) {
            4
        } else if n > 8 {
            n - 7
        } else {
            n
        }) + 1;

        println!(
            "count: {} - expected count: {}",
            block_finality.blocks_by_hash.len(),
            expected_count
        );

        assert_eq!(block_finality.blocks_by_hash.len(), expected_count);
        assert_eq!(block_finality.blocks_by_no.len(), expected_count);

        if n == 7 {
            assert_eq!(find_block_by_hash(&block_finality, &blocks_hashes[4]), 5);
            assert_eq!(
                find_block_by_hash(&block_finality, &blocks_hashes[1]),
                0xFFFFFFFF
            );
        }

        if n == 8 {
            rollback_to(&mut block_finality, &blocks_hashes[7]).unwrap();
        }

        let mut to_write = vec![];
        block_finality.serialize(&mut to_write).unwrap();

        let mut res_block_finality = BlockFinality::with_params(
            0,
            shard_ident.clone(),
            ShardStorage::new(test_storage.shard_storage(shard_ident.clone()).unwrap()),
            None,
        );

        println!("BBB");
        res_block_finality
            .deserialize(&mut Cursor::new(to_write))
            .unwrap();
        println!("CCC");
    }
}
