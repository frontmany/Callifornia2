import { Navigate, Route, Routes } from 'react-router-dom'
import { Toaster } from 'sonner'
import { LobbyRoute } from '@/routes/LobbyRoute'
import { LoginRoute } from '@/routes/LoginRoute'
import { RoomRoute } from '@/routes/RoomRoute'

function App() {
  return (
    <>
      <Routes>
        <Route path="/" element={<LoginRoute />} />
        <Route path="/lobby" element={<LobbyRoute />} />
        <Route path="/r/:roomId" element={<RoomRoute />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
      <Toaster richColors position="top-right" />
    </>
  )
}

export default App
