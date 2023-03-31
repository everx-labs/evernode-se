import "./Beta.sol";


contract Gamma {
    constructor() public {
        tvm.accept();
    }

    function getCallback(uint nonce) external pure {
        Beta(msg.sender).triggerCallback{flag: 64}(nonce);
    }
}
