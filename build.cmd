@echo off

set TON_NODE="tonlabs/ton-node"
set TONOS_SE="tonlabs/local-node"

set BIN_TARGET="ton_node_startup"

set Q_SERVER_GITHUB_REPO="https://github.com/tonlabs/ton-q-server"
set Q_SERVER_GITHUB_REV="master"

echo.
echo *** Building TON Node SE ***
echo.
docker build ^
    --build-arg BIN_TARGET="%BIN_TARGET%" ^
    --tag %TON_NODE% ^
    ton-node-se ^
    || exit /b

echo.
echo *** Building TONOS SE image ***
echo.
docker build ^
    --build-arg TON_NODE="%TON_NODE%" ^
    --build-arg Q_SERVER_GITHUB_REPO="%Q_SERVER_GITHUB_REPO%" ^
    --build-arg Q_SERVER_GITHUB_REV="%Q_SERVER_GITHUB_REV%" ^
    --tag %TONOS_SE% ^
    docker ^
    || exit /b

echo.
echo BUILD SUCCESS
echo.
echo How to run:
echo docker run -d --name local-node -e USER_AGREEMENT=yes -p80:80 %TONOS_SE%
