#!/bin/bash

# Gateway Zero 项目启动脚本

echo "=== Gateway Zero 启动脚本 ==="

# 0. 若 8080 已被旧的 gateway 占用，先停止（避免启动了新版本但端口仍指向旧进程）
echo "[0/2] 检查 8080 端口占用..."
PID=$(lsof -nP -t -iTCP:8080 -sTCP:LISTEN 2>/dev/null || true)
if [ -n "$PID" ]; then
  CMD=$(ps -p "$PID" -o comm= 2>/dev/null | tr -d ' ' || true)
  if [ "$CMD" = "gateway" ]; then
    echo "检测到旧 gateway 正在运行 (pid=$PID)，准备停止..."
    kill "$PID" || true
    sleep 1
  else
    echo "端口 8080 正被占用 (pid=$PID, cmd=$CMD)，请先释放端口后再启动。"
    exit 1
  fi
fi

# 1. 启动 PostgreSQL 数据库
echo "[1/2] 启动 PostgreSQL 数据库..."
docker start gateway-postgres
sleep 2

# 2. 启动 Rust 后端
echo "[2/2] 启动 Rust 后端服务..."
cd /home/Geoff001/Code/Project/Graduation_Project/gateway_zero
cargo run

echo ""
echo "=== 启动完成 ==="
echo "后端: http://localhost:8080/"
echo "管理端 UI：请在另一个终端启动 /home/Geoff001/Code/Project/captok（本仓库已移除内置旧前端）"
echo "按 Ctrl+C 停止服务"
