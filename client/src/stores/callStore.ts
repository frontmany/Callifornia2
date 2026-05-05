import { create } from 'zustand'

export type CallPhase =
  | 'idle'
  | 'connecting_connector'
  | 'authenticating'
  | 'connecting_signaling'
  | 'attaching'
  | 'in_room'
  | 'error'

type CallState = {
  phase: CallPhase
  roomId: string
  participants: string[]
  lastError: string | null
  setPhase: (phase: CallPhase) => void
  setRoomId: (roomId: string) => void
  setParticipants: (participants: string[]) => void
  addParticipant: (nickname: string) => void
  removeParticipant: (nickname: string) => void
  setError: (error: string | null) => void
}

export const useCallStore = create<CallState>((set) => ({
  phase: 'idle',
  roomId: '',
  participants: [],
  lastError: null,
  setPhase: (phase) => set({ phase }),
  setRoomId: (roomId) => set({ roomId }),
  setParticipants: (participants) => set({ participants }),
  addParticipant: (nickname) =>
    set((state) => ({ participants: Array.from(new Set([...state.participants, nickname])) })),
  removeParticipant: (nickname) =>
    set((state) => ({ participants: state.participants.filter((p) => p !== nickname) })),
  setError: (lastError) => set({ lastError }),
}))
