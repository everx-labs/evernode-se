import "./Beta.sol";


contract Alpha {
    address beta;

    constructor(address beta_addr) public {
        tvm.accept();
        beta = beta_addr;
    }

    function massTrigger(uint count) external view {
        tvm.accept();

        for (uint i = 0; i < count; i++) {
            Beta(beta).trigger{value: 0.3 ever}(i);
        }
    }
}
