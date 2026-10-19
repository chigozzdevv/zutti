FROM nvidia/cuda:12.8.1-devel-ubuntu24.04 AS build

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    build-essential ca-certificates clang cmake curl git libgmp-dev libssl-dev \
    pkg-config protobuf-compiler python3 unzip xz-utils \
    && rm -rf /var/lib/apt/lists/*
RUN curl --proto '=https' --tlsv1.2 -fsSLo /tmp/rustup-init \
    https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --profile minimal --default-toolchain 1.93.0 \
    && rm /tmp/rustup-init
ENV PATH="/root/.cargo/bin:/opt/risc0/bin:${PATH}"
ENV CARGO_BUILD_JOBS=8
RUN cargo install risc0-r0vm --version '=3.0.5' --locked \
    --features cuda,disable-dev-mode --bin r0vm --root /opt/risc0 \
    && cargo install rzup --version '=0.5.2' --locked --root /opt/risc0 \
    && rzup install risc0-groth16 0.1.0

WORKDIR /build
COPY repo/ ./
COPY programs/ /opt/zutti/programs/
ENV ZUTTI_PROGRAMS_DIR=/opt/zutti/programs
RUN cargo build --locked --release -p zutti-prover --bin source-prover \
    && install -m 755 target/release/source-prover /opt/risc0/bin/source-prover

FROM nvidia/cuda:12.8.1-runtime-ubuntu24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    ca-certificates libgmp10 libgomp1 libssl3t64 python3 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /opt/risc0/bin/r0vm /usr/local/bin/r0vm
COPY --from=build /opt/risc0/bin/source-prover /usr/local/bin/source-prover
COPY --from=build /root/.risc0/ /root/.risc0/
COPY run.py /opt/zutti/run.py
COPY build.json /opt/zutti/build.json
RUN r0vm --version && source-prover --help >/dev/null \
    && python3 -m py_compile /opt/zutti/run.py
ENV RISC0_DEV_MODE=0
ENV CUDA_VISIBLE_DEVICES=0
ENV TMPDIR=/workspace/tmp
WORKDIR /workspace
ENTRYPOINT ["python3", "/opt/zutti/run.py"]
CMD ["--help"]
