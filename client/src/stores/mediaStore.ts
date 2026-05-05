import { create } from 'zustand'

type MediaState = {
  localStream: MediaStream | null
  remoteStreams: Record<string, MediaStream>
  micEnabled: boolean
  camEnabled: boolean
  screenEnabled: boolean
  setLocalStream: (stream: MediaStream | null) => void
  setRemoteStream: (nickname: string, stream: MediaStream) => void
  removeRemoteStream: (nickname: string) => void
  setMicEnabled: (enabled: boolean) => void
  setCamEnabled: (enabled: boolean) => void
  setScreenEnabled: (enabled: boolean) => void
}

export const useMediaStore = create<MediaState>((set) => ({
  localStream: null,
  remoteStreams: {},
  micEnabled: true,
  camEnabled: true,
  screenEnabled: false,
  setLocalStream: (localStream) => set({ localStream }),
  setRemoteStream: (nickname, stream) =>
    set((state) => ({ remoteStreams: { ...state.remoteStreams, [nickname]: stream } })),
  removeRemoteStream: (nickname) =>
    set((state) => {
      const next = { ...state.remoteStreams }
      delete next[nickname]
      return { remoteStreams: next }
    }),
  setMicEnabled: (micEnabled) => set({ micEnabled }),
  setCamEnabled: (camEnabled) => set({ camEnabled }),
  setScreenEnabled: (screenEnabled) => set({ screenEnabled }),
}))
