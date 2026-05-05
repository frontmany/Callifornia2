import { env } from '@/config/env'
import { ConnectorClient } from '@/services/connector'
import { SignalingClient } from '@/services/signaling'
import { WebRtcClient } from '@/services/webrtc'
import { useAuthStore } from '@/stores/authStore'
import { useCallStore } from '@/stores/callStore'
import { useMediaStore } from '@/stores/mediaStore'
import { errorMap } from '@/types/errors'
import type { SignalingServerMessage } from '@/types/protocol'

class CallOrchestrator {
  private connector: ConnectorClient | null = null
  private signaling: SignalingClient | null = null
  private rtc: WebRtcClient | null = null

  async connectAndCreate() {
    await this.runConnectorFlow('create')
  }

  async connectAndJoin(roomId: string) {
    await this.runConnectorFlow('join', roomId)
  }

  leave() {
    const sessionId = useAuthStore.getState().sessionId
    const roomId = useCallStore.getState().roomId
    if (this.signaling && sessionId && roomId) {
      this.signaling.send({ type: 'leave', session_id: sessionId, room_id: roomId })
      this.signaling.send({ type: 'logout', session_id: sessionId })
    }
    this.rtc?.close()
    this.connector?.close()
    this.signaling?.close()
    useCallStore.getState().setPhase('idle')
    useCallStore.getState().setParticipants([])
  }

  async toggleMic(enabled: boolean) {
    await this.rtc?.toggleMic(enabled)
    useMediaStore.getState().setMicEnabled(enabled)
  }

  async toggleCam(enabled: boolean) {
    await this.rtc?.toggleCam(enabled)
    useMediaStore.getState().setCamEnabled(enabled)
  }

  async toggleScreen(enabled: boolean) {
    await this.rtc?.toggleScreen(enabled)
    useMediaStore.getState().setScreenEnabled(enabled)
  }

  private async runConnectorFlow(intent: 'create' | 'join', roomId?: string) {
    const auth = useAuthStore.getState()
    useCallStore.getState().setPhase('connecting_connector')

    this.connector = new ConnectorClient(env.connectorWsUrl, (msg) => {
      if (msg.type === 'auth_ok') {
        auth.setSessionId(msg.session_id)
        useCallStore.getState().setPhase('authenticating')
        if (intent === 'create') this.connector?.send({ type: 'create' })
        else if (roomId) this.connector?.send({ type: 'join', room_id: roomId })
      }
      if (msg.type === 'signaling_ready') {
        void this.connectSignaling(msg.signaling_url, msg.token, intent, roomId)
      }
    })

    this.connector.connect()
    this.connector.send({ type: 'auth', nickname: auth.nickname })
  }

  private async connectSignaling(
    signalingUrl: string,
    token: string,
    intent: 'create' | 'join',
    roomId?: string
  ) {
    useCallStore.getState().setPhase('connecting_signaling')
    this.signaling = new SignalingClient(signalingUrl, (msg) => void this.handleSignaling(msg))
    this.signaling.connect()

    const sessionId = useAuthStore.getState().sessionId
    this.signaling.send({ type: 'attach', token })

    this.rtc = new WebRtcClient(
      (desc) => {
        this.signaling?.send({
          type: 'sdp',
          session_id: sessionId,
          sdp: desc.sdp ?? '',
          sdp_type: desc.type as 'offer' | 'answer',
        })
      },
      (candidate) => {
        this.signaling?.send({
          type: 'candidate',
          session_id: sessionId,
          candidate: candidate.candidate,
          sdp_mid: candidate.sdpMid ?? '0',
        })
      },
      (remoteStream) => {
        const streamId = remoteStream.id || `remote-${Date.now()}`
        useMediaStore.getState().setRemoteStream(streamId, remoteStream)
      }
    )

    const localStream = await this.rtc.initLocalMedia()
    useMediaStore.getState().setLocalStream(localStream)

    if (intent === 'create') this.signaling.send({ type: 'create', session_id: sessionId })
    else if (roomId) this.signaling.send({ type: 'join', session_id: sessionId, room_id: roomId })
  }

  private async handleSignaling(msg: SignalingServerMessage) {
    const call = useCallStore.getState()
    if (msg.type === 'created') {
      call.setRoomId(msg.room_id)
      call.setParticipants([msg.your_nickname])
      call.setPhase('in_room')
      await this.rtc?.renegotiate()
    } else if (msg.type === 'joined') {
      call.setRoomId(msg.room_id)
      call.setParticipants(msg.participants)
      call.setPhase('in_room')
      await this.rtc?.renegotiate()
    } else if (msg.type === 'participant_joined') {
      call.addParticipant(msg.nickname)
      await this.rtc?.renegotiate()
    } else if (msg.type === 'participant_left') {
      call.removeParticipant(msg.nickname)
      useMediaStore.getState().removeRemoteStream(msg.nickname)
      await this.rtc?.renegotiate()
    } else if (msg.type === 'sdp') {
      await this.rtc?.handleRemoteSdp(msg.sdp, msg.sdp_type)
    } else if (msg.type === 'candidate') {
      await this.rtc?.addIceCandidate({ candidate: msg.candidate, sdpMid: msg.sdp_mid })
    } else if (msg.type === 'transfer_required') {
      await this.connectAndJoin(msg.room_id)
    } else if (msg.type === 'room_closed' || msg.type === 'service_unavailable') {
      this.leave()
    } else if (msg.type === 'error') {
      call.setError(errorMap[msg.code] ?? msg.message)
      if (msg.code === 'room_not_ready') {
        window.setTimeout(() => this.signaling?.send({ type: 'create', session_id: useAuthStore.getState().sessionId }), 1500)
      }
    }
  }
}

export const callOrchestrator = new CallOrchestrator()
