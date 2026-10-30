# Application background

Generated with the built-in image_gen tool on 2026-10-30, using the user's original character illustration as a reference for a new expanded landscape scene.

Prompt: Expand the reference into a wide 16:9 desktop application background. Preserve the long dark-haired girl with pale blue white ribbon headdress and turquoise ornaments, smiling with closed eyes while holding a teacup on a wooden veranda. Keep the character in the right third with safe margins around her face and headdress. Extend the left two-thirds into a quiet Japanese veranda and soft green sunlit garden, wooden railings, distant pavilion and gentle afternoon light. Match the delicate anime illustration style. Leave low-complexity light areas in the left and center for controls. No text, logos, UI widgets, seams or duplicated characters. Full bleed opaque landscape artwork.

Runtime behavior: embedded PNG, loaded once into a texture. Cover scaling preserves aspect ratio, crops excess edges and biases horizontal cropping toward the character. Translucent white UI surfaces preserve text contrast.

## 2026-10-30 服装与光影修正

使用内置 image_gen 编辑原背景，仅修正人物裙摆下方的服装覆盖和光影衔接。

最终提示词：对这张应用程序背景插画进行保守的服装一致性修复。人物整体着装覆盖完整、端庄。仅修正右下角浅蓝色长裙下方可见的少量衣物：用连续的白色裤袜衣料统一覆盖，采用柔和浅蓝灰阴影，让白色衣料与裙摆之间的光照衔接自然，消除突兀的肤色块与阴影断层。白色织物以平整柔和的色面表现，不刻画身体轮廓或肌肉细节，不增加身体展示。保持原有坐姿，允许裙摆自然遮挡不必要的细节。其余画面的表情、发型、发饰、上半身服装、茶具、木桌、庭院、构图和整体色调尽量保持原样。

## 2026-10-30 改用完整参考图并修复清晰度

完整原图保存在 background-reference.png。本次默认背景改为这张完整参考图的清晰度修复版，替代之前扩图和单独重画服装的版本。

使用内置 image_gen，最终提示词：对所附完整插画做高保真清晰度修复和超分辨率输出。只改善锯齿、压缩噪声、细线模糊和色阶断裂，不增加锐化白边。保留完整画幅、倾斜视角、圆形窗框曲线、木栏杆、地板、灯架、墙面、窗帘、书架、桌子、花瓶、茶具和地毯的位置与透视。人物表情、五官、头发轮廓、衣物、坐姿及原服装覆盖和光影保持原样，不更改人体形状、鞋袜或裙摆，不扩图，不补画场景，不添加文字、水印或装饰。
