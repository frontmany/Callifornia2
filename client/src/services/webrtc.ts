import { env } from '@/config/env'

type NegotiationHandler = (sdp: RTCSessionDescriptionInit) => void

type CandidateHandler = (candidate: RTCIceCandidate) => void

export class WebRtcClient {
  private pc: RTCPeerConnection
  private negotiationPending = false
  private localVideoSender: RTCRtpSender | null = null

  constructor(
    private readonly onNegotiation: NegotiationHandler,
    private readonly onCandidate: CandidateHandler,
    private readonly onRemoteTrack: (stream: MediaStream) => void
  ) {
    this.pc = new RTCPeerConnection({ iceServers: env.iceServers })
    this.pc.onicecandidate = (event) => {
      if (event.candidate) this.onCandidate(event.candidate)
    }
    this.pc.ontrack = (event) => {
      const stream = event.streams[0]
      if (stream) this.onRemoteTrack(stream)
    }
  }

  async initLocalMedia() {
    const stream = await navigator.mediaDevices.getUserMedia({ video: true, audio: true })
    for (const track of stream.getTracks()) {
      const sender = this.pc.addTrack(track, stream)
      if (track.kind === 'video') this.localVideoSender = sender
    }
    return stream
  }

  async renegotiate() {
    if (this.pc.signalingState !== 'stable' || this.negotiationPending) return
    this.negotiationPending = true
    try {
      const offer = await this.pc.createOffer({ iceRestart: false })
      await this.pc.setLocalDescription(offer)
      this.onNegotiation(offer)
    } finally {
      this.negotiationPending = false
    }
  }

  async handleRemoteSdp(sdp: string, sdpType: 'offer' | 'answer') {
    if (sdpType === 'answer') {
      await this.pc.setRemoteDescription({ type: 'answer', sdp })
      return
    }
    await this.pc.setRemoteDescription({ type: 'offer', sdp })
    const answer = await this.pc.createAnswer()
    await this.pc.setLocalDescription(answer)
    this.onNegotiation(answer)
  }

  async addIceCandidate(candidate: RTCIceCandidateInit) {
    await this.pc.addIceCandidate(candidate)
  }

  async toggleMic(enabled: boolean) {
    this.pc.getSenders().forEach((sender) => {
      if (sender.track?.kind === 'audio') sender.track.enabled = enabled
    })
  }

  async toggleCam(enabled: boolean) {
    this.pc.getSenders().forEach((sender) => {
      if (sender.track?.kind === 'video') sender.track.enabled = enabled
    })
  }

  async toggleScreen(enabled: boolean) {
    if (!this.localVideoSender) return
    if (enabled) {
      const stream = await navigator.mediaDevices.getDisplayMedia({ video: true })
      const track = stream.getVideoTracks()[0]
      await this.localVideoSender.replaceTrack(track)
      track.onended = () => {
        void this.toggleScreen(false)
      }
      return
    }
    const cam = await navigator.mediaDevices.getUserMedia({ video: true })
    const track = cam.getVideoTracks()[0]
    await this.localVideoSender.replaceTrack(track)
  }

  close() {
    this.pc.getSenders().forEach((s) => s.track?.stop())
    this.pc.close()
  }
}
