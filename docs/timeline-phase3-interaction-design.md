# Timeline Phase 3 — 素材交互状态设计

**状态：** Phase 3A 已实现（2026-11-28）；本文件保留完整 Phase 3 设计，音量线/淡入淡出仍属于 3B。
**基线：** `feat/timeline-zoom-phase2b`，`608a585`。Phase 2 已验收。附件 normal 参考图作为普通态基线，
其 header/thumbnail/waveform 结构保持不变。

> **Phase 3A 实现差异：** 选中边框统一为 **白色**（不是 accent）；仅 primary 显示左右手柄；
> 音频只做区域 hover 高亮，音量线/淡入淡出手柄留到 3B；禁用 clip 与隐藏轨道分离，禁用是 clip 级
> 变暗且不可交互，隐藏轨道是整条轨道不绘制；框选不做反选/自动滚动。
**约束：** ADR-0015 与 [UI Design System](14-ui-design-system.md)；不改变已验收布局、轨道高度、素材样式、
缩放、缩略图、波形和播放头；不实现拖拽/裁剪/分割/吸附/工程修改；T2/T3、零拷贝预览和其他面板不在范围。

---

## 1. 设计原则

1. **普通状态像素不变。** 未悬停、未选中的素材保持现有 body/thumbnail/waveform/label。
2. **状态是局部 UI state。** 悬停、选择集、框选、主选中项只存在于 `arc-app/src/timeline`；不写工程、
   不发命令、不触发解码。
3. **一个几何来源。** 素材矩形、header/thumb/audio 区域和命中区都由同一个 `clip_geometry()` 计算，
   绘制与命中测试不得各算一套。
4. **先命中，后绘制。** 每帧一次指针命中，按优先级产生一个 `ClipHover`；绘制只消费这个状态。
5. **时间轴是高频画布。** 不建立 per-clip widget 树；不因 hover 增加解码、纹理或持续重绘。
6. **颜色走语义角色。** 不新增散落的 hex；时间轴新增角色集中定义，普通状态继续使用现有色值/角色。

---

## 2. 现有几何基线（只读事实）

当前代码的固定尺寸，Phase 3 不改：

| 区域 | 值 |
|---|---:|
| 左侧控制列 `CONTROL_W` | 112 pt |
| Toolbar / Ruler / Scrollbar | 34 / 22 / 9 pt |
| Lane gap | 4 pt |
| Video / Image lane, clip | 76 / 72 pt |
| Audio lane, clip | 50 / 46 pt |
| Text / Effect / Sticker lane, clip | 24 / 20 pt |
| Video：header / thumb / embedded audio | 18 / 38 / 16 pt |
| Image：header / image | 18 / 54 pt |
| Audio：header / wave | 16 / 30 pt |
| Body 圆角 / 现有选中边框 | 4 pt / 1.5 pt inside |

现有行为：普通点击单选；选中边框画在 `visible` 矩形上；没有 hover、框选、多选和音频控件。
Phase 3 保留点击选中，但需把“可见矩形边框”改为“真实素材矩形按内容区裁剪”，避免在视口边缘画出假边框。

---

## 3. 状态模型

### 3.1 状态枚举

```text
ClipVisualState
  Normal
  Hover { zone: Body | TrimLeft | TrimRight | Audio | FadeIn | FadeOut | Volume }
  Selected { primary: bool }
  Locked
  Disabled        // 轨道隐藏 / 未来 clip.enabled == false
  Muted           // 仅音频区域
```

`Selected` 与 hover 可叠加：悬停不改变选择，只在选中态上增加 handle 高亮。`Locked`、`Disabled` 是
**覆盖态**，优先于选择；`Muted` 只覆盖音频子区域，不改变视频画面和选择。

### 3.2 状态优先级（从高到低）

1. **Active drag / marquee**（Phase 3 只有框选，没有拖拽素材）
2. **Fixed column / scrollbar / ruler / toolbar** —— 指针进入这些区域时，素材 hover 必须为 `None`
3. **Locked / Disabled** —— 不产生 trim/audio hover；locked 使用 `NotAllowed`
4. **TrimLeft / TrimRight** —— 素材真实左右边缘
5. **FadeIn / FadeOut** —— 音频区域内的菱形手柄
6. **Volume** —— 音频区域内的水平线
7. **Audio / Body** —— 素材主体
8. **Empty lane** —— 清空选择或开始框选

只允许一个 clip 获得 hover；相邻素材的 trim 命中冲突时，取指针距离更近的真实边缘；距离相同取左侧
素材的右边缘（保持“右边缘优先”的稳定规则）。

### 3.3 选择模型

```text
ClipRef { track: usize, clip: usize }
Selection { items: BTreeSet<ClipRef>, primary: Option<ClipRef> }
```

- `primary` 是最后加入的选中项，也是唯一显示 trim/audio 控件的素材。
- `Selected { primary: true }`：1.5 pt **白色**边框 + 左右手柄；手柄与边框分离 4 pt。
- `Selected { primary: false }`：同样 1.5 pt **白色**边框，无手柄。
- 选择集跨轨道；不使用 `Option<(usize, usize)>`。
- 锁定/隐藏轨道的 clip 不进入选择集；轨道锁定/隐藏时，从其选择集中移除并重选 primary。

---

## 4. 命中区域与鼠标样式

### 4.1 视频素材（72 pt，含音频）

```text
clip.left                                                              clip.right
┌──┬──────────────────────────────────────────────────────────────────┬──┐
│T │ Header 18: label                                                  │T │
│  ├──────────────────────────────────────────────────────────────────┤  │
│  │ Thumbnails 38                                                     │  │
│  ├──────────────────────────────────────────────────────────────────┤  │
│  │ Audio 16:   ◇──────────── volume line ────────────────◇           │  │
└──┴──────────────────────────────────────────────────────────────────┴──┘
 ↑ 6 trim        ↑ fade-in (inset 8)              fade-out (inset 8) ↑ 6 trim
```

| 区域 | 命中 | 视觉 | Cursor |
|---|---|---|---|
| 左/右 trim | 实际边缘向内 6 pt，全 clip 高；宽度 <20 pt 时缩为 4 pt，<12 pt 隐藏 | 悬停：2 pt accent 竖条；主选中：3 pt 竖条，圆角 1.5，上下各留 4 pt | `ResizeHorizontal` |
| Fade in/out | 菱形中心 inset 8 pt，12×12 pt 命中 | 7×7 pt 菱形 `handle.fade`；hover/主选中显示 | `Grab`（未来拖动时 `Grabbing`） |
| Volume | 音量线上下 3 pt，x 从 left+12 到 right-12；可见宽 <24 pt 时隐藏 | 1.25 pt 水平线 `handle.volume`；hover/主选中显示 | `ResizeVertical` |
| Audio body | 剩余 audio 区 | audio 区 +4% 白、波形提亮 | `Default` |
| Body | 其余可见主体 | 内侧 1 pt hover 描边 + 2% 提亮 | `Default` |
| Locked | 整个可见 body | 8% 黑罩 + 右上锁图标，无 handle | `NotAllowed` |

### 4.2 图片（72 pt）

```text
┌──────────────────────────────────────────────┐
│ Header 18                                    │
├──────────────────────────────────────────────┤
│ Image 54                                     │
└──────────────────────────────────────────────┘
 ↑ trim 6                                 6 ↑
```

只有 trim + body；没有 audio/fade/volume。Hover 图片区域整体提亮 3%，边缘同视频。

### 4.3 音频（46 pt）

```text
┌──────────────────────────────────────────────┐
│ Header 16                                    │
├──────────────────────────────────────────────┤
│ Wave 30:  ◇──────── volume line ────────◇    │
└──────────────────────────────────────────────┘
 ↑ trim 6                                 6 ↑
```

与视频 audio 子区域相同，但 wave 更高，音量线在 wave 垂直中心，菱形位于 wave 顶部 +3 pt。
`Muted` 时波形降到 45% 不透明度、线/菱形隐藏，右侧显示静音标记；trim 仍可用。

### 4.4 文字 / 特效 / 贴纸（20 pt）

```text
┌──────────────────────────────────────────────┐
│ T / effect / sticker label                   │
└──────────────────────────────────────────────┘
 ↑ trim 4（窄素材）                      4 ↑
```

只有 body + trim；没有 audio、fade、volume、thumbnail。trim 命中 4 pt，视觉 2 pt；
宽度 <12 pt 时只保留 body 选择，不显示 trim。标签仍按现有规则贴在可见左边缘。

### 4.5 视口边缘、滚动和极限缩放

- 命中使用的可见区：`clip_rect ∩ content`，其中 `content = body - 左侧控制列 - 滚动条`。
- **trim 只画在素材真实边缘。** 当真实边缘在视口外（或落在固定列/滚动条下）时，不画、不命中；
  绝对不能在视口边缘生成假手柄。
- 选中/hover 边框画在**真实 clip 矩形**上，再由 painter 裁剪到 content；不是画在 `visible` 上。
- 标签仍固定在可见左边缘（现有行为）。
- 极端缩放：trim 固定 6 pt 屏幕宽度，不随时长缩放；fade/volume 固定屏幕尺寸。
- 窄素材：可见宽 <20 pt 时 trim 4 pt；<12 pt 时无 trim；<4 pt 时只保留 1 pt 选中边，不显示标签。
- 相邻素材边界：两个 trim 重叠时取指针更近的边；主选中素材优先。
- 横向滚动/缩放导致几何变化时，丢弃上一帧 hover 并重新命中，不缓存屏幕坐标。
- 空白 lane 框选：`Crosshair`；框选中的指针不触发素材 hover。

---

## 5. 各素材类型的状态设计

| 类型 | Normal | Hover | Selected | Locked / Disabled | Muted | 部分超出视口 |
|---|---|---|---|---|---|---|
| Video（有音频） | 现有 header/thumb/wave | body 描边；边缘 2 pt；audio 区提亮 | 1.5 pt 边框 + trim + audio 控件 | 锁图标/NotAllowed；隐藏则 35% 整体 | audio 区 45%，控件隐藏 | 只显示真实可见部分；边缘不在视口才无 handle |
| Video（无音频） | header + 拉满 thumb | 同 body/trim | 边框 + trim，无 audio 控件 | 同上 | 不适用 | 同上 |
| Image | header + image | 图片区 +3%，边缘 | 边框 + trim | 同上 | 不适用 | 同上 |
| Audio | header + wave | wave 提亮；边缘；控件出现 | 边框 + trim + fade/volume | 同上 | wave 45%，控件隐藏 | 同上 |
| Text | 现有 label | 内侧 1 pt 描边 | 边框 + 4 pt trim | 锁图标/禁用 | 不适用 | 标签贴可见左边 |
| Effect | 现有 label | 同上 | 同上 | 同上 | 不适用 | 同上 |
| Sticker | 现有 label | 同上 | 同上 | 同上 | 不适用 | 同上 |

“禁用”本阶段映射为**轨道隐藏**（`track.visible == false`）；未来 clip 级 `enabled` 复用同一视觉。
“静音”只影响音频区域；视频画面、选择、trim 不受影响。

---

## 6. 状态切换规则

| 事件 | 结果 |
|---|---|
| 指针进入 clip body | `Hover::Body`；不改变选择 |
| 指针进入真实边缘 | `Hover::TrimLeft/Right`，优先于 body/audio；显示 ghost 2 pt 竖条 |
| 指针进入 audio 区 | `Hover::Audio`；显示 waveform 提亮和音频控件 |
| 指针进入 fade/volume | 对应 hover；优先于 audio body |
| 左键单击 body（无修饰） | 单选该 clip；若它已在多选中，收敛为单选 |
| Ctrl/Cmd + 左键单击 | 切换该 clip；加入时成为 primary；移除 primary 时选最后一个仍选中项 |
| Shift + 左键单击 | 本阶段不做范围选择；保留给未来（见 §11） |
| 左键单击空白 lane | 清空选择 |
| 空白 lane 拖拽 | 框选；默认替换选择，Shift/Ctrl/Cmd 追加 |
| 框选释放 | 选中与 marquee 可见矩形相交的未锁定、未隐藏 clip；空框清空 |
| Esc | 有框选则取消并恢复框选前选择；否则清空选择 |
| 点击 locked/disabled clip | 不改变选择；cursor `NotAllowed`；不显示 handle |
| 轨道锁定/隐藏 | 立即从选择集移除该轨道 clip；primary 重选 |
| 轨道静音 | 选择不变；audio 控件隐藏，wave 降透明 |
| 滚动/缩放/窗口 resize | 清空 hover，按新几何重新命中；选择保留 |
| 选择集变化 | 只 request 一次 repaint；不触发解码、不修改工程 |

框选起点必须在空白 lane 内（不能在 clip、ruler、固定列、滚动条上）。Phase 3 不做框选自动滚动。
框选矩形裁剪到 content；填充 `accent.muted` 12%，边框 1 pt `accent`。

---

## 7. 视觉规格

普通状态完全沿用现有 `draw_clip_body`、缩略图、波形、标签和 missing 状态。新增状态只加轻量 overlay：

| 状态 | 视觉 |
|---|---|
| Hover body | 内侧 1 pt `accent.hover` 55% 描边；body 2% 白 overlay；不改变圆角 |
| Hover edge | 2 pt `accent` 竖条，距上下各 4 pt；不显示完整手柄 |
| Selected primary | 1.5 pt 白色 inside 边框 + 3 pt 白色 trim 手柄（内缩 4 pt） |
| Selected secondary | 1.5 pt 白色 inside 边框；无手柄 |
| Fade handle | 7×7 菱形 `accent`，中心点；hover 提亮到 `accent.hover` |
| Volume line | 1.25 pt `accent.muted`，hover/selected 时 70% → 100% |
| Locked | 8% 黑罩 + 右上 6 pt 锁图标；无 hover 提亮 |
| Disabled/hidden | 整体 35% 不透明度；无命中 |
| Muted | audio 区 45% 不透明度 + 6 pt 静音图标；wave/线/手柄隐藏 |
| Marquee | `accent.muted` 12% 填充 + 1 pt `accent` 边框 |

动效：hover、选择、handle 均使用 `motion.instant`；时间轴不播放过渡动画，避免持续重绘。
高对比主题下，边框/handle 使用 `focus-ring` 或 `accent` 的强对比色；颜色不是唯一信号
（锁、静音、菱形/竖条/水平线形状分别区分）。

---

## 8. 需要新增或复用的公共组件

### 8.1 复用

- `draw_clip_body`、`draw_clip_outline`、`draw_clip_label`、`draw_missing`、`draw_waveform_bars`、
  `uv_fill`、`icon_button`、`draw_track_controls`、`draw_scrollbar`。
- 现有 `egui::Response`、`Sense`、`Painter`、`CursorIcon`；不新建第二个 UI 框架。
- 现有 `Track.locked/visible/muted` 状态。

### 8.2 时间轴本地新增（不进入通用组件库）

| 组件/类型 | 职责 |
|---|---|
| `ClipRef` / `Selection` | 稳定引用、主选中项、Ctrl 切换、框选结果、锁定/隐藏过滤 |
| `ClipGeometry` | 一次算出 rect/visible/header/thumb/audio/wave，以及每个命中区 |
| `ClipHover` / `HitZone` | 当前 hover 的 clip + zone；全帧唯一 |
| `hit_test_clip` | 纯函数；优先级 trim > fade > volume > audio > body；处理窄素材、视口、相邻边 |
| `Marquee` | 起点、当前点、additive、取消快照；矩形绘制与相交结果 |
| `draw_clip_chrome` | 在内容/标签之后绘制 hover 边框、trim、fade、volume、锁/禁用/静音 overlay |
| `draw_trim_handle` / `draw_fade_handle` / `draw_volume_line` | 三种不同形状的绘制原语，避免混淆 |
| `draw_marquee` | 框选 overlay |

### 8.3 语义 token

新增到共享 token/theme 层的时间轴角色（若共享 `ui/tokens.rs` 尚未落地，先在 `timeline/style.rs`
集中定义，后续原样迁入，不在绘制调用处散写 hex）：

`timeline.clip-hover`、`timeline.clip-selected`、`timeline.clip-selected-secondary`、
`timeline.trim`、`timeline.fade`、`timeline.volume`、`timeline.locked-veil`、
`timeline.disabled-opacity`、`timeline.marquee-fill`、`timeline.marquee-border`。

不新增通用 `TimelineClip` 组件到 `ui/components/`：设计系统明确把 timeline clip 排除在第一阶段组件库外，
且时间轴必须保持虚拟化绘制。

---

## 9. 实现范围

**做：**

1. `Selection` 替换现有 `selected: Option<(usize, usize)>`；支持单选、Ctrl/Cmd 多选、框选、清空。
2. 一个纯命中函数 + 一个 `ClipHover`；hover 状态仅由指针位置产生。
3. 绘制 hover 边框、ghost trim、主选中 trim、audio fade/volume、locked/disabled/muted overlay。
4. 修正选中/hover 边框按真实 clip 矩形绘制并裁剪，不再画在 `visible` 上。
5. 单元测试：几何、优先级、窄素材、视口/固定列、Ctrl/框选/Esc、锁定/静音。
6. 视觉测试：正常态像素不变；1f、极限缩小、横向滚动、部分遮挡、150%/200% DPI。

**不做：**

- 拖拽移动、trim 拖动、split、ripple/roll、snap、link 的实际修改。
- 任何工程/Engine 命令、undo、保存、播放头或播放行为修改。
- 右键菜单、双击重命名、键盘范围选择、框选自动滚动。
- T2/T3、零拷贝预览、其他面板、通用组件库扩建。

---

## 10. 验收标准

1. **普通态不变：** 未 hover/未选中时，video/image/audio/text/effect/sticker 的 body、缩略图、波形、
   标签、圆角、颜色与 Phase 2 截图一致（允许因选中语义新增的 overlay 之外无差异）。
2. **Hover 区域正确：** body、左右真实边缘、audio 区、fade、volume 各自触发正确视觉和 cursor；
   离开后立即恢复；不误触相邻 clip。
3. **单选：** 1.5 pt 边框 + 主选中 trim；视频有音频时显示 audio 控件；图片/文字/特效/贴纸不显示
   audio 控件。
4. **多选：** Ctrl/Cmd 切换；所有选中项有边框；只有 primary 显示手柄；点击空白或 Esc 清空；
   框选替换、Shift/Ctrl 追加；框选不选中 locked/hidden。
5. **音频区分：** trim 是竖条、fade 是菱形、volume 是水平线；三者命中区域不重叠；
   muted 时 fade/volume 隐藏、wave 降透明；无音频视频不显示任何 audio 控件。
6. **类型差异：** 文字/特效/贴纸 20 pt 使用 4 pt trim 且无 audio；图片无 audio；音频 46 pt。
7. **特殊状态：** locked 无手柄/NotAllowed；hidden 35% 且无命中；muted 仅影响 audio；
   部分超出视口时无假边缘/假手柄，标签仍贴可见左边。
8. **几何：** 24/25/30/29.97/59.94、12 px/s 到 200 px/帧、横向/纵向滚动、固定列遮挡下命中一致；
   窄素材按阈值降级。
9. **性能：** 不新增 per-clip widget 树、hover 不触发解码、不持续 repaint；8 轨/2,000 可见 clip 秒
   paint ≤ 8 ms；现有 timeline 测试全部通过。
10. **范围：** 没有实际编辑操作、没有 T2/T3/零拷贝/其他面板改动。

---

## 11. 尚未确认的交互细节

1. **锁定素材是否可选？** 推荐：不可选、无手柄、`NotAllowed`；仅 hover 显示锁提示。
2. **多选中点击已选素材**：推荐收敛为单选；Ctrl 点击才 toggle。
3. **框选修饰键**：推荐普通框选替换、Shift/Ctrl 追加；是否需要“Ctrl 框选 toggle”未定。
4. **所有选中项还是仅 primary 显示手柄？** 推荐仅 primary，避免多套手柄重叠。
5. **音频控件默认可见性**：推荐仅 hover/主选中显示；未选中且未 hover 时保持参考图外观。
6. **fade 手柄位置/形状**：推荐 wave/audio 顶部内缩 8 pt 的 7 pt 菱形；是否改成小方块待确认。
7. **音量线基准**：本阶段没有真实 gain 字段，建议显示 0 dB 示意线；Phase 4 再接工程值。
8. **20 pt 文字/特效/贴纸是否显示 trim**：推荐显示 4 pt trim；也可选择本阶段不给小素材 trim。
9. **“禁用”定义**：当前建议映射到 `track.visible == false`；未来 clip 级 `enabled` 是否复用同一视觉。
10. **是否需要多选计数/状态提示**：本阶段不做，避免新增 chrome；可在状态栏另议。
11. **框选是否自动滚动**：推荐 Phase 3 不做，Phase 4 与拖拽一起设计。
12. **右键菜单、双击、键盘范围选择**：全部留给后续阶段。

---

## 12. 停止点

本文件只定义设计，没有修改任何产品代码。等待确认后再进入实现。
