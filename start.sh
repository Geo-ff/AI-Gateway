#!/bin/bash

# Gateway Zero 项目启动脚本

echo "=== Gateway Zero 启动脚本 ==="

# 1. 启动 PostgreSQL 数据库
echo "[1/3] 启动 PostgreSQL 数据库..."
docker start gateway-postgres
sleep 2

# 2. 启动 Rust 后端
echo "[2/3] 启动 Rust 后端服务..."
cd /home/Geoff001/Code/Project/Graduation_Project/gateway_zero
cargo run &

sleep 3

# 3. 启动 Vue 前端
echo "[3/3] 启动 Vue 前端服务..."
cd /home/Geoff001/Code/Project/Graduation_Project/gateway_zero/frontend
pnpm dev &

echo ""
echo "=== 启动完成 ==="
echo "前端: http://localhost:5173/"
echo "后端: http://localhost:8080/"
echo "按 Ctrl+C 停止服务"

wait
