/**
 * WebGPU waveform renderer (DESIGN.md §6): the frame's rectangles ({@link frameRects}) drawn as
 * instances of one quad, premultiplied over a transparent canvas. {@link WebGpuRenderer.create}
 * answers `null` where WebGPU is missing, and the timeline keeps the Canvas2D renderer.
 */
import { frameRects, RECT_FLOATS } from './rects'
import type { FrameDescriptor, RenderTokens, WaveformRenderer } from './types'

const SHADER = /* wgsl */ `
struct Uniforms { size: vec2f }
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct Varyings {
  @builtin(position) position: vec4f,
  @location(0) color: vec4f,
}

@vertex
fn vertexMain(
  @builtin(vertex_index) corner: u32,
  @location(0) rect: vec4f,
  @location(1) color: vec4f,
) -> Varyings {
  let unit = vec2f(f32(corner & 1u), f32(corner >> 1u));
  let pixel = rect.xy + unit * rect.zw;
  var out: Varyings;
  out.position = vec4f(pixel.x / uniforms.size.x * 2.0 - 1.0, 1.0 - pixel.y / uniforms.size.y * 2.0, 0.0, 1.0);
  out.color = color;
  return out;
}

@fragment
fn fragmentMain(in: Varyings) -> @location(0) vec4f {
  return in.color;
}
`

/** `GPUBufferUsage` flags, which TypeScript's DOM library does not declare. */
const USAGE = { COPY_DST: 0x08, VERTEX: 0x20, UNIFORM: 0x40 } as const

export class WebGpuRenderer implements WaveformRenderer {
  private instances: GPUBuffer | null = null
  private lost = false

  private constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly device: GPUDevice,
    private readonly context: GPUCanvasContext,
    private readonly pipeline: GPURenderPipeline,
    private readonly uniforms: GPUBuffer,
    private readonly bindGroup: GPUBindGroup,
    /** Called once when the GPU goes away, so the timeline can draw with Canvas2D instead. */
    onLost: () => void,
  ) {
    void device.lost.then(() => {
      this.lost = true
      onLost()
    })
  }

  /** A renderer on `canvas`, or `null` when this WebView has no usable WebGPU. */
  static async create(
    canvas: HTMLCanvasElement,
    onLost: () => void,
  ): Promise<WebGpuRenderer | null> {
    const gpu = (navigator as Navigator & { gpu?: GPU }).gpu
    if (!gpu) return null
    try {
      const adapter = await gpu.requestAdapter()
      if (!adapter) return null
      const device = await adapter.requestDevice()
      const context = canvas.getContext('webgpu') as GPUCanvasContext | null
      if (!context) return null
      const format = gpu.getPreferredCanvasFormat()
      context.configure({ device, format, alphaMode: 'premultiplied' })
      const module = device.createShaderModule({ code: SHADER })
      const pipeline = device.createRenderPipeline({
        layout: 'auto',
        vertex: {
          module,
          entryPoint: 'vertexMain',
          buffers: [
            {
              arrayStride: RECT_FLOATS * 4,
              stepMode: 'instance',
              attributes: [
                { shaderLocation: 0, offset: 0, format: 'float32x4' },
                { shaderLocation: 1, offset: 16, format: 'float32x4' },
              ],
            },
          ],
        },
        fragment: {
          module,
          entryPoint: 'fragmentMain',
          targets: [
            {
              format,
              blend: {
                color: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha', operation: 'add' },
                alpha: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha', operation: 'add' },
              },
            },
          ],
        },
        primitive: { topology: 'triangle-strip' },
      })
      const uniforms = device.createBuffer({
        size: 16,
        usage: USAGE.UNIFORM | USAGE.COPY_DST,
      })
      const bindGroup = device.createBindGroup({
        layout: pipeline.getBindGroupLayout(0),
        entries: [{ binding: 0, resource: { buffer: uniforms } }],
      })
      return new WebGpuRenderer(canvas, device, context, pipeline, uniforms, bindGroup, onLost)
    } catch (error) {
      console.warn('WebGPU is not usable here; drawing with Canvas2D', error)
      return null
    }
  }

  draw(frame: FrameDescriptor, tokens: RenderTokens): void {
    if (this.lost) return
    const { canvas, device } = this
    const pixelWidth = Math.max(1, Math.round(frame.width * frame.dpr))
    const pixelHeight = Math.max(1, Math.round(frame.height * frame.dpr))
    if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
      canvas.width = pixelWidth
      canvas.height = pixelHeight
    }
    const { data, count } = frameRects(frame, tokens)
    const bytes = count * RECT_FLOATS * 4
    if (bytes > 0 && (!this.instances || this.instances.size < bytes)) {
      this.instances?.destroy()
      this.instances = device.createBuffer({
        size: Math.max(bytes, 1) * 2,
        usage: USAGE.VERTEX | USAGE.COPY_DST,
      })
    }
    if (this.instances && bytes > 0) {
      device.queue.writeBuffer(this.instances, 0, data.buffer, data.byteOffset, bytes)
    }
    device.queue.writeBuffer(this.uniforms, 0, new Float32Array([pixelWidth, pixelHeight, 0, 0]))

    const encoder = device.createCommandEncoder()
    const pass = encoder.beginRenderPass({
      colorAttachments: [
        {
          view: this.context.getCurrentTexture().createView(),
          clearValue: { r: 0, g: 0, b: 0, a: 0 },
          loadOp: 'clear',
          storeOp: 'store',
        },
      ],
    })
    // Rows end at the sticky ruler.
    const clipTop = Math.min(pixelHeight, Math.max(0, Math.round(frame.clipTop * frame.dpr)))
    pass.setScissorRect(0, clipTop, pixelWidth, Math.max(0, pixelHeight - clipTop))
    if (this.instances && count > 0) {
      pass.setPipeline(this.pipeline)
      pass.setBindGroup(0, this.bindGroup)
      pass.setVertexBuffer(0, this.instances)
      pass.draw(4, count)
    }
    pass.end()
    device.queue.submit([encoder.finish()])
  }

  dispose(): void {
    this.instances?.destroy()
    this.uniforms.destroy()
    this.device.destroy()
  }
}
