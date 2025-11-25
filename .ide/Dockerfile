# .ide/Dockerfile
FROM rust:1.91.1

RUN sed -i 's@deb.debian.org@mirrors.cloud.tencent.com@g' /etc/apt/sources.list.d/debian.sources

# Install additional development tools and dependencies
RUN apt-get update && \
    apt-get install -y cmake clang llvm-dev libclang-dev \
            protobuf-compiler \
            wget unzip lsof nload htop net-tools dnsutils openssh-server vim && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

# Install nvm and Node.js 22
ENV NVM_DIR=/root/.nvm
ENV NVM_NODEJS_ORG_MIRROR=https://mirrors.tuna.tsinghua.edu.cn/nodejs-release/
RUN curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash && \
    . "$NVM_DIR/nvm.sh" && \
    nvm install 22 && \
    npm install -g @openai/codex @anthropic-ai/claude-code @cometix/ccline && \
    ln -s "$NVM_DIR/versions/node/$(nvm current)/bin/"* /usr/local/bin/

# Install Rust development VS Code extensions for code-server
RUN curl -fsSL https://code-server.dev/install.sh | sh \
    && code-server --install-extension rust-lang.rust-analyzer \
    && code-server --install-extension vadimcn.vscode-lldb \
    && code-server --install-extension serayuzgur.crates \
    && code-server --install-extension formulahendry.code-runner \
    && code-server --install-extension tamasfe.even-better-toml

ENV RUST_BACKTRACE=1
ENV CARGO_INCREMENTAL=1
ENV LANG=C.UTF-8
ENV LANGUAGE=C.UTF-8
