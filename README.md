# git-branch-check

批量检测文件夹下所有 Git 仓库的分支是否包含最新主分支提交的命令行工具。

## 功能

- 递归遍历所有子目录，找到每个 Git 仓库
- 对每个仓库自动 fetch 远程更新
- 自动识别实际主分支名（通过 `origin/HEAD`、或探测 main/master/develop/trunk）
- 比较当前分支与远程主分支的提交差，计算落后数
- 输出包含相对路径、当前分支、落后提交数的表格
- 只要有一个仓库超过阈值就返回错误码 1，便于 CI 集成

## 安装

```bash
cargo build --release
```

## 用法

```bash
# 扫描当前目录
git-branch-check

# 扫描指定目录
git-branch-check /path/to/projects

# 设置阈值（默认 10，任一仓库超过即返回错误码）
git-branch-check -t 5 /path/to/projects

# 跳过 fetch（离线模式）
git-branch-check --no-fetch /path/to/projects
```

## 输出示例

```
+---------------------+----------------+--------+
| Relative Path       | Current Branch | Behind |
+---------------------+----------------+--------+
| services/api        | feature/login  | 3      |
| services/worker     | main           | 0      |
| libs/shared         | dev            | 15     |
+---------------------+----------------+--------+

ERROR: 1 repo(s) behind by more than 10 commits:
  libs/shared [dev] behind 15
```

## CI 集成

退出码：
- `0`: 所有仓库都在阈值内
- `1`: 至少有一个仓库落后超过阈值
- `2`: 指定路径不存在

```yaml
# GitHub Actions 示例
- name: Check branch freshness
  run: git-branch-check -t 10 ./repos
```
