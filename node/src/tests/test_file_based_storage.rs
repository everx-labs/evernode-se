use std::{path::PathBuf, time::Instant};

use crate::data::{FSKVStorage, FSStorage, ShardStorage};
use crate::tests::{
    generate_block_with_seq_no, get_block, save_block, save_shard_state, shard_state,
};
use ever_block::{BlkPrevInfo, HashmapAugType, Serializable, ShardIdent, ShardStateUnsplit, UInt256};

#[test]
fn test_create_directory_tree() {
    let mut path = PathBuf::from("../target");
    let _fbs = FSStorage::new(path.clone()).unwrap();
    path.push("workchains");

    assert!(path.as_path().exists());
}

#[test]
fn test_save_and_restore_shard_state() {
    let shard = ShardIdent::with_prefix_len(2, 1, 0xC000_0000_0000_0000).unwrap();
    let fbs = ShardStorage::new(Box::new(
        FSKVStorage::with_path(shard, PathBuf::from("../target")).unwrap(),
    ));

    let shard_ident = ShardIdent::with_prefix_len(1, 0, 0xABCD_EF12_0000_0000).unwrap();

    let mut ss = ShardStateUnsplit::default();
    ss.set_shard(shard_ident.clone());
    save_shard_state(&fbs, &ss).unwrap();

    let ss_restored = shard_state(&fbs).unwrap();

    assert_eq!(ss, ss_restored);

    let shard_ident2 = ShardIdent::with_prefix_len(2, 0, 0xF0000000_00000000).unwrap();
    ss.set_shard(shard_ident2.clone());

    save_shard_state(&fbs, &ss).unwrap();

    let ss_restored = shard_state(&fbs).unwrap();

    assert_eq!(ss, ss_restored);
}

#[test]
fn test_save_and_restore_blocks() {
    let shard_ident = ShardIdent::with_prefix_len(2, 0, 0xF0000000_00000000).unwrap();

    let fbs = ShardStorage::new(Box::new(
        FSKVStorage::with_path(shard_ident.clone(), PathBuf::from("../target")).unwrap(),
    ));

    let mut blocks = vec![];
    let mut prev_info = BlkPrevInfo::default();

    println!("start test...");

    for n in 0..10 {
        let now = Instant::now();
        // todo: 5000
        let block = generate_block_with_seq_no(shard_ident.clone(), n + 1, prev_info, 500);
        let d = now.elapsed();

        let mut count = 0;
        block
            .read_extra()
            .unwrap()
            .read_account_blocks()
            .unwrap()
            .iterate_objects(|acc_block| {
                count += acc_block.transaction_count()?;
                Ok(true)
            })
            .unwrap();
        println!("transaction count {}", count);
        println!(
            "generate_block time elapsed sec={}.{} ",
            d.as_secs(),
            d.subsec_nanos()
        );

        let now = Instant::now();

        save_block(&fbs, &block, UInt256::ZERO).unwrap();
        let d = now.elapsed();
        println!(
            "save_block time elapsed sec={}.{} ",
            d.as_secs(),
            d.subsec_nanos()
        );

        let now = Instant::now();
        blocks.push(block);
        let d = now.elapsed();
        println!(
            "blocks.push time elapsed sec={}.{} ",
            d.as_secs(),
            d.subsec_nanos()
        );

        prev_info = BlkPrevInfo::default();
    }

    //thread::sleep(Duration::from_millis(500));

    for n in 0..10 {
        let now = Instant::now();
        let block = get_block(&fbs, n + 1, 0).unwrap();
        let d = now.elapsed();
        println!(
            "get_block time elapsed sec={}.{} ",
            d.as_secs(),
            d.subsec_nanos()
        );

        let bin1 = block.write_to_bytes().unwrap();
        /*BagOfCells::with_root(SliceData::from(Arc::<CellData>::from(
        block.write_to_new_cell().unwrap())))
        .write_to(&mut bin1, false).unwrap();*/

        let bin2 = blocks[n as usize].write_to_bytes().unwrap();
        /*BagOfCells::with_root(SliceData::from(Arc::<CellData>::from(
        blocks[n as usize].write_to_new_cell().unwrap())))
        .write_to(&mut bin2, false).unwrap();*/

        assert_eq!(bin1, bin2);
    }
}
