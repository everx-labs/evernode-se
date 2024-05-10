use std::{thread, time, sync::Arc};
use ever_block::*;
use crate::engine::InMessagesQueue;
use crate::tests::{shard_state_info_deserialize, shard_state_info_with_params};

fn get_message(n: u8) -> Message {

    let mut msg = Message::with_int_header(
        InternalMessageHeader::with_addresses(
            MsgAddressInt::with_standart(None, -1, AccountId::from_raw(vec![n,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0], 256)).unwrap(),
            MsgAddressInt::with_standart(None, -1, AccountId::from_raw(vec![0,n,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0], 256)).unwrap(),
            CurrencyCollection::default()
        )
    );
    msg.set_at_and_lt(0, n as u64);

    let mut stinit = StateInit::default();
    stinit.set_special(TickTock::with_values(false, true));
    let code = SliceData::new(vec![n, 0b11111111,0b11111111,0b11111111,0b11111111,0b11111111,0b11111111,0b11110100]);
    stinit.set_code(code.into_cell());
    let data = SliceData::new(vec![0b00111111, n,0b11111111,0b11111111,0b11111111,0b11111111,0b11111111,0b11110100]);
    stinit.set_data(data.into_cell());
    let library = SliceData::new(vec![0b00111111, 0b11111111,n,0b11111111,0b11111111,0b11111111,0b11111111,0b11110100]);
    stinit.set_library(library.into_cell());

    let body = SliceData::new(
            vec![n,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,
                 0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,
                 0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,
                 0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,
                 0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,
                 0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,
                 0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,
                 0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,
                 0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,
                 0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,
                 0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0xFE,0x80]);

    msg.set_state_init(stinit);
    msg.set_body(body);

    msg
}


#[test]
fn test_in_messages_queue_sigle_thread() {
    let queue = InMessagesQueue::new(100);

    let m1 = get_message(1);
    let m2 = get_message(2);
    let m3 = get_message(3);
    queue.queue(m1.clone()).unwrap();
    queue.queue(m2.clone()).unwrap();
    queue.queue(m3.clone()).unwrap();
    assert_eq!(m1, queue.dequeue(-1).unwrap());
    assert_eq!(m2, queue.dequeue(-1).unwrap());
    assert_eq!(m3, queue.dequeue(-1).unwrap());
}

#[test]
fn test_in_messages_queue_multi_thread() {
    let queue = Arc::new(InMessagesQueue::new(100));

    let queue1 = queue.clone();
    let queue2 = queue.clone();
    let queue3 = queue.clone();

    let m1 = get_message(1);
    let m1_ = m1.clone();
    let m2 = get_message(2);
    let m2_ = m2.clone();
    let m3 = get_message(3);
    let m3_ = m3.clone();
    let m4 = get_message(4);
    let m4_ = m4.clone();
    let m5 = get_message(5);
    let m5_ = m5.clone();
    let m6 = get_message(6);
    let m6_ = m6.clone();
    queue.queue(m1.clone()).unwrap();

    let handler1 = thread::spawn(move || {
        assert_eq!(m1_, queue1.dequeue(-1).unwrap());
        queue1.queue(m2.clone()).unwrap();
    });
    handler1.join().unwrap();

    let handler2 = thread::spawn(move || {
        thread::sleep(time::Duration::from_millis(1000));
        assert_eq!(m2_, queue2.dequeue(-1).unwrap());
        queue2.queue(m3).unwrap();
        queue2.queue(m4).unwrap();
    });
    handler2.join().unwrap();

    let handler3 = thread::spawn(move || {
        thread::sleep(time::Duration::from_millis(2000));

        assert_eq!(m3_, queue3.dequeue(-1).unwrap());
        assert_eq!(m4_, queue3.dequeue(-1).unwrap());

        queue3.queue(m5).unwrap();
        queue3.queue(m6).unwrap();
    });
    handler3.join().unwrap();

    assert_eq!(m5_, queue.dequeue(-1).unwrap());
    assert_eq!(m6_, queue.dequeue(-1).unwrap());
}

#[test]
fn test_shard_chain_inf0_serialization() {
    let ssi = shard_state_info_with_params(121, 234123, UInt256::from([56;32]));

    let data = ssi.serialize();

    let ssi2 = shard_state_info_deserialize(&mut std::io::Cursor::new(data)).unwrap();

    assert_eq!(ssi, ssi2);
}

#[test]
fn test_dequeue() {
    let queue = InMessagesQueue::new(100);

    let msg1 = get_message(1);
    queue.queue(msg1.clone()).unwrap();
    let msg2 = get_message(2);
    queue.queue(msg2.clone()).unwrap();
    let msg3 = get_message(3);
    queue.queue(msg3.clone()).unwrap();

    let msg = queue.dequeue(-1).unwrap();
    assert_eq!(msg, msg1.clone());
    let msg = queue.dequeue(-1).unwrap();
    assert_eq!(msg, msg2.clone());
    let msg = queue.dequeue(-1).unwrap();
    assert_eq!(msg, msg3.clone());

    assert!(queue.dequeue(0).is_none());

    queue.queue(msg2.clone()).unwrap();
    queue.queue(msg3.clone()).unwrap();

    let msg = queue.dequeue(-1).unwrap();
    assert_eq!(msg, msg2);
    let msg = queue.dequeue(-1).unwrap();
    assert_eq!(msg, msg3);
}

#[test]
fn test_queue_overflow() {
    let queue = InMessagesQueue::new(3);

    let msg1 = get_message(1);
    queue.queue(msg1.clone()).unwrap();

    let msg2 = get_message(2);
    queue.queue(msg2.clone()).unwrap();

    let msg3 = get_message(3);
    queue.queue(msg3.clone()).unwrap();

    let msg4 = get_message(4);

    assert!(queue.is_full());
    assert!(queue.queue(msg4.clone()).is_err());

    queue.dequeue(-1).unwrap();

    assert!(!queue.is_full());
    assert!(queue.queue(msg1.clone()).is_ok());
    assert!(queue.queue(msg4.clone()).is_err());
}
