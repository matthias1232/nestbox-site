# illustrations/

Decorative WebP illustrations and the optional login loop for the console. Generated, not drawn; prompts are committed so they can be rendered again. Rendered 2026-11-03 (illustrations paths follow package D03; design doc mentions `assets/illustrations/`).

| Asset | Prompt file | Size | Tool / model |
|---|---|---|---|
| `login.webp` | `../assets/prompts/login.txt` | 1600x900, 58 KB | `render_image` / `image-gen-combo` |
| `empty-sandboxes.webp` | `../assets/prompts/empty-sandboxes.txt` | 1024x1024, 37 KB | `render_image` / `image-gen-combo` |
| `empty-nodes.webp` | `../assets/prompts/empty-nodes.txt` | 1024x1024, 49 KB | `render_image` / `image-gen-combo` |
| `empty-models.webp` | `../assets/prompts/empty-models.txt` | 1024x1024, 68 KB | `render_image` / `image-gen-combo` |
| `docs-tenant-tree.webp` | `../assets/prompts/docs-tenant-tree.txt` | 1600x900, 66 KB | `render_image` / `image-gen-combo` |
| `docs-model-cluster.webp` | `../assets/prompts/docs-model-cluster.txt` | 1600x900, 71 KB | `render_image` / `image-gen-combo` |
| `docs-policy-path.webp` | `../assets/prompts/docs-policy-path.txt` | 1600x900, 75 KB | `render_image` / `image-gen-combo` |
| `login-loop.mp4` | `../assets/prompts/login-loop.txt` | 3 s, 16:9, 720p, 775 KB | `render_video` / `video-gen-combo` |

Renders came out larger than the budget and were resized with ImageMagick (`convert -resize ... -quality 80`).
