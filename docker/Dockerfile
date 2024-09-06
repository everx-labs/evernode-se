ARG TON_NODE

FROM ${TON_NODE} as ton-node

FROM ubuntu:24.04 as final

ARG Q_SERVER_GITHUB_REPO
ARG Q_SERVER_GITHUB_REV

ENV TZ="Europe/Moscow"
ENV LC_ALL=C.UTF-8
ENV LANG=C.UTF-8
ENV DEBIAN_FRONTEND=noninteractive
ENV HOME="/root"
ENV NVM_DIR="$HOME/.nvm"
ENV PATH="$HOME/.nvm/versions/node/v18.20.4/bin:$NVM_DIR:$HOME/.local/bin:$PATH"
ENV LANG=en_US.UTF-8
ENV PYTHONIOENCODING=utf-8

COPY --from=ton-node /node /node

ADD q-server /q-server
ADD ton-node /ton-node
ADD Procfile /honcho-cfg/
ADD arango /arango

RUN apt update; \
    apt upgrade -y; \
    apt install -y python3 \
        apt-utils\
        python3 \
        python3-pip \
        gpg \
        dos2unix \
        build-essential \
        nginx \
        git \
        curl; \
    gpg --receive-keys EA93F5E56E751E9B; \
    gpg --export -o arango.gpg EA93F5E56E751E9B; \
    mv arango.gpg /etc/apt/trusted.gpg.d/arango.gpg; \
    echo 'deb [Signed-By=/etc/apt/trusted.gpg.d/arango.gpg] https://download.arangodb.com/arangodb37/DEBIAN/ /' | tee /etc/apt/sources.list.d/arangodb.list; \
    apt update; \
    echo arangodb3 arangodb3/password password '' | debconf-set-selections; \
    echo arangodb3 arangodb3/password_again password '' | debconf-set-selections; \
    apt install -y arangodb3=3.7.18-1; \
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
    git clone --recursive --branch $Q_SERVER_GITHUB_REV $Q_SERVER_GITHUB_REPO ever-q-server; \
    cd ever-q-server; \
    npm ci; \
    npm run tsc; \
    npm ci --production; \
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

WORKDIR /honcho-cfg
RUN /arango/entrypoint init
COPY nginx.conf.d /etc/nginx/nginx.conf
COPY ton-live/web /var/www

EXPOSE 80
EXPOSE 3000 

ENTRYPOINT ["honcho"]
CMD ["start"]
