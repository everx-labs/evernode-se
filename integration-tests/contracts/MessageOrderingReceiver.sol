pragma solidity >0.5.0;

import "IMessageOrderingReceiver.sol";

contract MessageOrderingReceiver is IMessageOrderingReceiver {
    uint32 m_lastAccepted;

    constructor() public {
        require(msg.pubkey() == tvm.pubkey(), 101);
        tvm.accept();

        m_lastAccepted = 0;
    }

    function reset() public override {
        tvm.accept();

        m_lastAccepted = 0;
    }

    function acceptMessage(uint32 value) public override {
        require(msg.sender != address(0), 102);
        require(value == m_lastAccepted + 1, 103);
        tvm.accept();

        m_lastAccepted = value;
    }

    function getLastAccepted() public view returns (uint32 result) {
        result = m_lastAccepted;
    }
}