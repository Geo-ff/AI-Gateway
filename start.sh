#!/bin/bash

# Gateway Zero 项目启动脚本

echo "=== Gateway Zero 启动脚本 ==="

# 1. 启动 PostgreSQL 数据库
echo "[1/2] 启动 PostgreSQL 数据库..."
docker start gateway-postgres
sleep 2

# 2. 启动 Rust 后端
echo "[2/2] 启动 Rust 后端服务..."
cd /home/Geoff001/Code/Project/Graduation_Project/gateway_zero
cargo run &

sleep 3

echo ""
echo "=== 启动完成 ==="
echo "后端: http://localhost:8080/"
echo "管理端 UI：请在另一个终端启动 /home/Geoff001/Code/Project/captok（本仓库已移除内置旧前端）"
echo "按 Ctrl+C 停止服务"

wait
