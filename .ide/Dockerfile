FROM rust:1.91.1

RUN sed -i 's@deb.debian.org@mirrors.cloud.tencent.com@g' /etc/apt/sources.list.d/debian.sources

# Install additional development tools and dependencies
RUN apt-get update && \
    apt-get install -ycmake clang llvm-dev libclang-dev \
            protobuf-compiler \
            wget unzip lsof nload htop net-tools dnsutils openssh-server vim && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

# Install fnm and Node.js 22
RUN curl -o- https://fnm.vercel.app/install | bash && \
    export PATH="/root/.local/share/fnm:$PATH" && \
    eval "$(fnm env)" && \
    fnm install 22 && \
    fnm use 22 && \
    npm install -g @openai/codex @anthropic-ai/claude-code @cometix/ccline

# Make fnm and node available in all shells
ENV PATH="/root/.local/share/fnm:$PATH"
RUN echo 'eval "$(fnm env)"' >> /root/.bashrc

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
