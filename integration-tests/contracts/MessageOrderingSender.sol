pragma solidity >0.5.0;

import "IMessageOrderingReceiver.sol";

contract MessageOrderingSender {
    address m_receiver;
    
    constructor(address receiver) public acceptOnlyOwner {
        m_receiver = receiver;
    }

    modifier acceptOnlyOwner {
        require(msg.pubkey() == tvm.pubkey(), 101);
        tvm.accept();
        _;
    }

    function sendMessages(uint32 max) public acceptOnlyOwner {
        IMessageOrderingReceiver(m_receiver).reset.value(1e7)();
        for (uint32 val = 1; val <= max; val++) {
            IMessageOrderingReceiver(m_receiver).acceptMessage.value(1e7)(val);
        }
    }
}
