export type WsHandler<T> = (message: T) => void

export class TypedWebSocket<TOutbound, TInbound> {
  private ws: WebSocket | null = null
  private queue: TOutbound[] = []
  private watchdog: number | null = null

  constructor(
    private readonly url: string,
    private readonly parse: (raw: string) => TInbound,
    private readonly onMessage: WsHandler<TInbound>,
    private readonly onClose?: () => void
  ) {}

  connect() {
    this.ws = new WebSocket(this.url)
    this.ws.onopen = () => {
      this.flushQueue()
      this.touch()
    }
    this.ws.onmessage = (ev) => {
      this.touch()
      const data = this.parse(String(ev.data))
      this.onMessage(data)
    }
    this.ws.onclose = () => {
      this.clearWatchdog()
      this.onClose?.()
    }
  }

  send(payload: TOutbound) {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      this.queue.push(payload)
      return
    }
    this.ws.send(JSON.stringify(payload))
  }

  close() {
    this.clearWatchdog()
    this.ws?.close()
    this.ws = null
  }

  private flushQueue() {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return
    for (const msg of this.queue.splice(0)) {
      this.ws.send(JSON.stringify(msg))
    }
  }

  private touch() {
    this.clearWatchdog()
    this.watchdog = window.setTimeout(() => this.close(), 30_000)
  }

  private clearWatchdog() {
    if (this.watchdog) window.clearTimeout(this.watchdog)
    this.watchdog = null
  }
}
