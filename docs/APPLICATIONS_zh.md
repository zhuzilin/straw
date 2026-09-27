# 应用集成

[English](APPLICATIONS.md)

straw 核心负责字节、引用、持久化任务、结果接收和存储生命周期。应用负责 sample 的含义、组 batch、奖励、模型执行、worker 部署和 checkpoint。消费者可以是预处理流水线、推理服务或训练系统，相应框架的 adapter 放在使用 straw 的应用中。

## 集成边界

| straw | 应用 |
|---|---|
| `Record` 与带版本的 codec 名称 | 编解码自定义 sample/session 对象 |
| `TensorRef`、依赖、带校验的按行读取 | 张量形状、mask、路由和分数语义 |
| Lease、receipt 与持久化 continuation 引用 | Worker 调度、RPC 和重试策略 |
| 消费者状态与 ready batch 引用 | Batch 规划、分片与转换 |
| 保留所有者与封存 pack 的 GC | 真实读取完成信号及 checkpoint 释放 |
| 经外部 fencing 后重启的 WAL 恢复 | 模型/优化器/RNG 恢复与已提交 checkpoint 的选择 |

通过已有控制通道交换引用。JSON 引用类型独立于 Ray 和应用自身的 Python 类。自定义元数据使用显式带标记的 schema；遇到未知版本应报错，不应静默导入或 pickle 任意类。

每个进程保留长期使用的 writer。大张量字段单独发布，多个结果队列共享它们时通过 dependencies 描述。使用 `from_record_set_many` 批量恢复张量描述符。中间状态增量持久化，避免反复复制不断增长的整个缓冲区。

在 rollout/训练集成中，所有必要消费者完成后才确认原始输出已使用完；所有训练 rank 读完后才确认 ready batch。路由/分数采集不完整时，即使 token 生成完成，也不能发布为持久化 continuation；应从最近一个内部一致的输入重试。这些校验规则属于应用。

后台 GC 失败时记录该错误、停止接收新任务，并将错误传播给应用监督进程。保留不确定状态的存储用于排查。已经在远端运行的任务仍需要显式监督与 fencing。

联合 checkpoint 应先保留 pending、buffered、prefetched 数据的全部依赖，确保模型/优化器/RNG 和队列视图组件持久化，再发布指明精确版本的最终已提交 manifest。新 checkpoint 持久化之前保留上一份完整 checkpoint。仅模型的 latest 标记或队列消费确认无法替代这一协议。回滚后重做尚未提交的优化器步骤，与重复接收同一个逻辑结果是不同问题。

应用集成测试应覆盖真实 schema、中断/续跑、共享张量生命周期、多 reader 完成信号和整个任务恢复。这些测试补充[核心协议验证](VERIFICATION_zh.md)，不能替代它。
