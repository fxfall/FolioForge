# Canonical Semantic Equivalence Fixtures

这些夹具只用于公开的 Semantic IR 回归断言，不进入生产 Semantic IR，也不把 EPUB 当成唯一正确答案。

- `basic/manifest.json`：跨 TXT/Markdown/HTML/FB2/DOCX/EPUB 的公共基础语义。
- `rich/manifest.json`：脚注、Ruby、MathML、Figure、Alt、嵌套列表、表格和锚点的能力声明。
- DOCX/EPUB 的包夹具由测试辅助代码按 manifest 生成到外部临时目录，避免把二进制测试产物写入源码树。

比较对象是测试专用的 SemanticSnapshot：reading order、metadata、heading tree、paragraph text、list structure、note/link relationship、Ruby projection、Math、resource use。NodeId、ResourceId 数值、来源文件名、生成锚点、ZIP 路径和 CSS 序列化不参与等价判断。
