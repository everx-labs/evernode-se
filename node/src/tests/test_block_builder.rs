use rand::Rng;
use std::{thread, time::{Duration, Instant}};
use ever_block::*;
use crate::tests::{builder_add_test_transaction, builder_is_empty, builder_with_shard_ident};

macro_rules! println_elapsed{
    ( $msg: expr, $time: expr ) => {
        println!("{}: time elapsed sec={}.{:03}", $msg, $time.as_secs(), $time.subsec_millis())
    }
}

#[test]
fn test_blockbuilder_empty() {
    let bb = builder_with_shard_ident(ShardIdent::masterchain(), 1, BlkPrevInfo::default(), 0);
    let (block, _) = bb.finalize_block().unwrap();
    println!("{:?}", block);
}

fn build_transaction(acc: AccountId) -> (InMsg, OutMsg, OutMsg) {
    let rcv = AccountId::from_raw((0..32).map(|_| { rand::random::<u8>() }).collect::<Vec<u8>>(), 256);
    let mut imh = InternalMessageHeader::with_addresses (
        MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
        MsgAddressInt::with_standart(None, 0, rcv).unwrap(),
        CurrencyCollection::with_grams(10202)
    );

    // inmsg
    imh.ihr_fee = 10u64.into();
    imh.fwd_fee = 5u64.into();
    let mut inmsg1 = Message::with_int_header(imh);
    inmsg1.set_body(SliceData::new(vec![0x21;120]));

    // transaction
    let mut transaction = Transaction::with_address_and_status(acc.clone(), AccountStatus::AccStateActive);
    transaction.write_in_msg(Some(&CommonMessage::Std(inmsg1.clone()))).unwrap();

    let inmsg_int = InMsg::Immediate(InMsgFinal::with_cells(
        ChildCell::with_cell(MsgEnvelope::with_message_and_fee(&inmsg1, 9u64.into()).unwrap().serialize().unwrap()),
        ChildCell::with_cell(transaction.serialize().unwrap()),
        11u64.into(),
    ));

    let eimh = ExternalInboundMessageHeader {
        src: MsgAddressExt::with_extern(SliceData::new(vec![0x23, 0x52, 0x73, 0x00, 0x80])).unwrap(),
        dst: MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
        import_fee: 10u64.into(),
    };

    let mut inmsg = Message::with_ext_in_header(eimh);
    inmsg.set_body(SliceData::new(vec![0x01;120]));

    // inmsg
    let inmsgex = InMsg::External(InMsgExternal::with_cells(
        ChildCell::with_cell(inmsg.serialize().unwrap()),
        ChildCell::with_cell(transaction.serialize().unwrap())
    ));

    // outmsgs
    let mut imh = InternalMessageHeader::with_addresses(
        MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
        MsgAddressInt::with_standart(None, 0, AccountId::from_raw(vec![255;32], 256)).unwrap(),
        CurrencyCollection::with_grams(10202)
    );
    imh.ihr_fee = 10u64.into();
    imh.fwd_fee = 5u64.into();
    let outmsg1 = Message::with_int_header(imh);

    let eomh = ExtOutMessageHeader::with_addresses (
        MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
        MsgAddressExt::with_extern(SliceData::new(vec![0x23, 0x52, 0x73, 0x00, 0x80])).unwrap()
    );

    let mut outmsg2 = Message::with_ext_out_header(eomh);
    outmsg2.set_body(SliceData::new(vec![0x02;120]));

    let tr_cell: Cell = transaction.serialize().unwrap();

    let out_msg1 = OutMsg::New(OutMsgNew::with_cells(
        ChildCell::with_cell(MsgEnvelope::with_message_and_fee(&outmsg1, 9u64.into()).unwrap().serialize().unwrap()),
        ChildCell::with_cell(tr_cell.serialize().unwrap())
    ));

    let out_msg2 = OutMsg::External(OutMsgExternal::with_cells(
        ChildCell::with_cell(outmsg2.serialize().unwrap()),
        ChildCell::with_cell(tr_cell.serialize().unwrap())
    ));

    (if rand::thread_rng().gen() { inmsg_int } else { inmsgex }, out_msg1, out_msg2)
}

#[test]
fn test_blockbuilder_generate_blocks() {
    let total = Instant::now();
    let now = Instant::now();

    let mut block_builder = builder_with_shard_ident(ShardIdent::masterchain(), 1, BlkPrevInfo::default(), 0);
    println!("<--- Run without threads");
    let mut generate = Duration::new(0, 0);
    let mut append = generate;
    for _ in 0..1000 {
        let acc = AccountId::from_raw((0..32).map(|_| { rand::random::<u8>() }).collect::<Vec<u8>>(), 256);
        let now = Instant::now();
        let (in_msg, out_msg1, out_msg2) = build_transaction(acc.clone());
        generate += now.elapsed();
        builder_add_test_transaction(&mut block_builder, in_msg, &[out_msg1, out_msg2]).unwrap();
        append += now.elapsed();
    }
    append -= generate;
    println_elapsed!("generate", generate);
    println_elapsed!("append", append);
    if !builder_is_empty(&block_builder) {
        let d = now.elapsed();
        println_elapsed!("generate_block", d);

        let (block, _) = block_builder.finalize_block().unwrap();
        let d = now.elapsed();
        println_elapsed!("generate_block finalize", d);
        println!("!!!!!!!!!!BLOCK!!!!!!!!!");
        println!("block number {}", block.read_info().unwrap().seq_no());
        println!("block in msg count {}", block.read_extra().unwrap().read_in_msg_descr().unwrap().len().unwrap());
        println!("block out msg count {}", block.read_extra().unwrap().read_out_msg_descr().unwrap().len().unwrap());

        // serialize block
        let now = Instant::now();
        let mut builder = BuilderData::new();
        block.write_to(&mut builder).unwrap();

        let d = now.elapsed();
        println_elapsed!("write block to builder", d);

        let now = Instant::now();
        let buffer = ever_block::write_boc(&builder.into_cell().unwrap()).unwrap();

        let d = now.elapsed();
        println_elapsed!("serialize block to bag", d);

        // deserialize block

        let now = Instant::now();
        let block_cells_restored = ever_block::boc::read_single_root_boc(&buffer).unwrap();

        let d = now.elapsed();
        println_elapsed!("deserialize block from bag", d);

        let now = Instant::now();
        let mut block_restored = Block::default();
        block_restored.read_from(&mut SliceData::load_cell(block_cells_restored).unwrap()).unwrap();

        let d = now.elapsed();
        println_elapsed!("deserialize block", d);

        assert_eq!(1, block.read_info().unwrap().seq_no());
        assert!(!block.read_extra().unwrap().read_in_msg_descr().unwrap().is_empty());
    } else {
        println!("Block builder is empty.");
    }

    let d = total.elapsed();
    println_elapsed!("total", d);

    thread::sleep(Duration::from_millis(10));
}
