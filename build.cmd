@echo off

set TON_NODE="tonlabs/ton-node"
set TONOS_SE="tonlabs/evernode-se"

set BIN_TARGET="evernode_se"

set Q_SERVER_GITHUB_REPO="https://github.com/tonlabs/ton-q-server"
set /P Q_SERVER_GITHUB_REV=<q-server.version

echo.
echo *** Building Evernode SE ***
echo.
docker build ^
    --no-cache ^
    --build-arg BIN_TARGET="%BIN_TARGET%" ^
    --tag %TON_NODE% ^
    node ^
    || exit /b

echo.
echo *** Building TONOS SE image ***
echo.
docker build ^
    --no-cache ^
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
echo docker run -d --name evernode-se -e USER_AGREEMENT=yes -p80:80 %TONOS_SE%
