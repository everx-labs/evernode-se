import "./Gamma.sol";

contract Beta {
    uint public lowest_nonce = 0;
    address public gamma;

    constructor(address gamma_addr) public {
        tvm.accept();
        gamma = gamma_addr;
    }

    function trigger(uint nonce) external view {
        Gamma(gamma).getCallback{flag: 64}(nonce);
    }

    function triggerCallback(uint nonce) external {
        require (nonce == lowest_nonce, 9999);
        lowest_nonce++;
    }
}
