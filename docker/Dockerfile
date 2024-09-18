ARG TON_NODE

FROM ${TON_NODE} AS ton-node

FROM ubuntu:24.04 AS builder
ENV TZ="Europe/Moscow"
ENV LC_ALL=C.UTF-8
ENV LANG=C.UTF-8
ENV DEBIAN_FRONTEND=noninteractive
ENV HOME="/root"
ENV NVM_DIR="$HOME/.nvm"
ENV PATH="$HOME/.nvm/versions/node/v18.20.4/bin:$NVM_DIR:$HOME/.local/bin:$PATH"
ENV LANG=en_US.UTF-8
ENV PYTHONIOENCODING=utf-8

RUN apt update; \
    apt upgrade -y; \
    apt install -y \
        apt-utils \
        build-essential \
        git \
        curl;\
    rm -rf /var/lib/apt/lists/*; \
    touch .bashrc; \
    curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.0/install.sh | bash && \
    . "$NVM_DIR/nvm.sh" && \
    . "$NVM_DIR/bash_completion"; \
    nvm install 18 && \
    node -v; \
    npm -v;

FROM builder AS qserver

ARG Q_SERVER_GITHUB_REPO
ARG Q_SERVER_GITHUB_REV

RUN git clone --recursive --branch $Q_SERVER_GITHUB_REV $Q_SERVER_GITHUB_REPO /root/ever-q-server;

WORKDIR /root/ever-q-server

RUN npm ci; \
    npm run tsc; \
    npm ci --production;

FROM ubuntu:24.04 AS pre-final

COPY --from=ton-node /node /node
COPY --from=qserver /root/ever-q-server /root/ever-q-server

ADD q-server /q-server
ADD ton-node /ton-node
ADD Procfile /honcho-cfg/
ADD arango /arango

ENV TZ="Europe/Moscow"
ENV LC_ALL=C.UTF-8
ENV LANG=C.UTF-8
ENV DEBIAN_FRONTEND=noninteractive
ENV HOME="/root"
ENV NVM_DIR="$HOME/.nvm"
ENV PATH="$HOME/.nvm/versions/node/v18.20.4/bin:$NVM_DIR:$HOME/.local/bin:$PATH"
ENV LANG=en_US.UTF-8 PYTHONIOENCODING=utf-8

RUN apt update; \
    apt upgrade -y; \
    apt install -y \
        apt-utils \
        python3 \
        python3-pip \
        gpg \
        dos2unix \
        nginx \
        curl; \
    curl https://download.arangodb.com/arangodb37/DEBIAN/Release.key -o ~/Release.gpg; \
    gpg --keyid-format long --list-options show-keyring ~/Release.gpg; \
    gpg --import ~/Release.gpg; \
    curl https://download.arangodb.com/arangodb37/DEBIAN/amd64/arangodb3_3.7.18-1_amd64.deb.asc -o ~/arangodb3_3.7.18-1_amd64.deb.asc; \
    curl https://download.arangodb.com/arangodb37/DEBIAN/amd64/arangodb3_3.7.18-1_amd64.deb -o ~/arangodb3_3.7.18-1_amd64.deb; \
    echo arangodb3 arangodb3/password password '' | debconf-set-selections; \
    echo arangodb3 arangodb3/password_again password '' | debconf-set-selections; \
    gpg --verify ~/arangodb3_3.7.18-1_amd64.deb.asc ~/arangodb3_3.7.18-1_amd64.deb && dpkg -i ~/arangodb3_3.7.18-1_amd64.deb; \
    rm ~/Release.gpg ~/arango*; \
    rm -rf /var/lib/apt/lists/*; \
    groupadd -r -g 103 nginx; \ 
    useradd -r -p '' -G 103 -d /nonexistent -g "users" -s /bin/false -u 103 nginx; \
    mkdir -p /run/nginx; \
    cd /root; \
    touch .bashrc; \
    curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.0/install.sh | bash && \
    . "$NVM_DIR/nvm.sh" && \
    . "$NVM_DIR/bash_completion"; \
    nvm install 18 && \
    node -v; \
    npm -v; \
    nvm cache clean; \
    npm install --global foxx-cli; \
    rm -rf /etc/nginx/conf.d/; \
    rm -rf /etc/nginx/v.host/; \
    rm -f /etc/nginx/nginx.conf; \
    rm -f "/usr/lib/python$(python3 --version | grep -P -o '\d+\.\d+')/EXTERNALLY-MANAGED"; \
    pip install --upgrade pip; \
    pip install -Iv honcho==1.0.1; \
    mkdir -p '/var/lib/arangodb3'; \
    mkdir -p '/var/lib/arangodb3-apps'; \
    chmod +w '/var/lib/arangodb3'; \
    chmod +w '/var/lib/arangodb3-apps'; \
    dos2unix /q-server/entrypoint /arango/entrypoint /arango/config /ton-node/entrypoint /honcho-cfg/Procfile; \
    chmod +x /q-server/entrypoint; \
    chmod +x /arango/entrypoint; \
    chmod +x /node/entrypoint;

WORKDIR /

FROM ubuntu:24.04 AS final

COPY --from=pre-final /etc /etc
COPY --from=pre-final /var /var
COPY --from=pre-final /usr /usr
COPY --from=pre-final /q-server /q-server
COPY --from=pre-final /arango /arango
COPY --from=pre-final /node /node
COPY --from=pre-final /ton-node /ton-node
COPY --from=pre-final /root /root
COPY --from=pre-final /honcho-cfg /honcho-cfg
COPY nginx.conf.d /etc/nginx/nginx.conf
COPY ton-live/web /var/www

ENV TZ="Europe/Moscow"
ENV LC_ALL=C.UTF-8
ENV LANG=C.UTF-8
ENV DEBIAN_FRONTEND=noninteractive
ENV HOME="/root"
ENV NVM_DIR="$HOME/.nvm"
ENV PATH="$HOME/.nvm/versions/node/v18.20.4/bin:$NVM_DIR:$HOME/.local/bin:$PATH"
ENV LANG=en_US.UTF-8 PYTHONIOENCODING=utf-8

WORKDIR /honcho-cfg
RUN /arango/entrypoint init;
EXPOSE 80
EXPOSE 3000 

ENTRYPOINT ["honcho"]
CMD ["start"]
