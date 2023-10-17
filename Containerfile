FROM ghcr.io/cross-rs/x86_64-unknown-linux-gnu:latest

RUN apt-get update && \
    apt-get install -y libssl-dev unzip

RUN curl -L -o /tmp/protoc.zip \
    https://github.com/protocolbuffers/protobuf/releases/download/v24.4/protoc-24.4-linux-x86_64.zip && \
    unzip /tmp/protoc.zip -d /opt/protoc && \
    rm -f /tmp/protoc.zip && \
    ln -s /opt/protoc/bin/protoc /usr/local/bin/protoc
