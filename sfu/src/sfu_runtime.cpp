#include "sfu_runtime.h"

#include <chrono>
#include <utility>

SfuRuntime::SfuRuntime(std::shared_ptr<EventRouter> eventRouter)
    : m_eventRouter(std::move(eventRouter)), m_rtcConfig(loadIceConfiguration()) {}

std::shared_ptr<Room> SfuRuntime::getOrCreateRoom(const std::string& roomId) {
    auto it = m_rooms.find(roomId);
    if (it != m_rooms.end()) {
        return it->second;
    }
    auto room = std::make_shared<Room>(roomId);
    m_rooms.emplace(roomId, room);
    return room;
}

bool SfuRuntime::addPeer(const std::string& roomId,
                         const std::string& participantId,
                         const std::string& signalingId,
                         std::string& errorMessage) {
    if (roomId.empty() || participantId.empty()) {
        errorMessage = "room_id and participant_id must be non-empty";
        return false;
    }

    std::lock_guard lock(m_mutex);
    auto room = getOrCreateRoom(roomId);
    if (room->findPeer(participantId)) {
        errorMessage = "peer already exists";
        return false;
    }
    auto session = std::make_shared<PeerSession>(roomId, participantId, signalingId);
    if (!room->addPeer(session)) {
        errorMessage = "failed to add peer";
        return false;
    }
    try {
        session->initPeerConnection(m_eventRouter, m_rtcConfig);
    } catch (const std::exception& ex) {
        errorMessage = ex.what();
        session->shutdownPeerConnection();
        room->removePeer(participantId);
        if (room->peerCount() == 0) {
            m_rooms.erase(roomId);
        }
        return false;
    }
    return true;
}

bool SfuRuntime::removePeer(const std::string& roomId, const std::string& participantId) {
    if (roomId.empty() || participantId.empty()) {
        return false;
    }

    std::shared_ptr<PeerSession> session;
    {
        std::lock_guard lock(m_mutex);
        auto roomIt = m_rooms.find(roomId);
        if (roomIt == m_rooms.end()) {
            return false;
        }
        session = roomIt->second->findPeer(participantId);
    }
    if (session) {
        session->shutdownPeerConnection();
    }
    {
        std::lock_guard lock(m_mutex);
        auto roomIt = m_rooms.find(roomId);
        if (roomIt == m_rooms.end()) {
            return false;
        }
        const bool removed = roomIt->second->removePeer(participantId);
        if (roomIt->second->peerCount() == 0) {
            m_rooms.erase(roomIt);
        }
        return removed;
    }
}

std::shared_ptr<PeerSession> SfuRuntime::findPeerSession(const std::string& roomId,
                                                         const std::string& participantId) const {
    if (roomId.empty() || participantId.empty()) {
        return nullptr;
    }
    std::lock_guard lock(m_mutex);
    auto roomIt = m_rooms.find(roomId);
    if (roomIt == m_rooms.end()) {
        return nullptr;
    }
    return roomIt->second->findPeer(participantId);
}

std::optional<std::string> SfuRuntime::findSignalingId(const std::string& roomId,
                                                     const std::string& participantId) const {
    auto session = findPeerSession(roomId, participantId);
    if (!session) {
        return std::nullopt;
    }
    return session->signalingId();
}

sfu::SFUEvent SfuRuntime::buildHeartbeatEvent() const {
    sfu::SFUEvent event;
    event.set_room_id("");
    event.set_participant_id("");

    auto* hb = event.mutable_heartbeat();

    hb->set_timestamp(
        std::chrono::duration_cast<std::chrono::seconds>(
            std::chrono::system_clock::now().time_since_epoch())
            .count());

    hb->set_active_peers(static_cast<uint32_t>(activePeerCount()));
    hb->set_active_rooms(static_cast<uint32_t>(activeRoomCount()));

    return event;
}

std::size_t SfuRuntime::activePeerCount() const {
    std::lock_guard lock(m_mutex);
    std::size_t n = 0;
    for (const auto& [_, room] : m_rooms) {
        n += room->peerCount();
    }
    return n;
}

std::size_t SfuRuntime::activeRoomCount() const {
    std::lock_guard lock(m_mutex);
    return m_rooms.size();
}

bool SfuRuntime::sendEvent(const std::string& roomId,
                           const std::string& participantId,
                           const sfu::SFUEvent& event) const {
    if (!m_eventRouter) {
        return false;
    }
    const auto signalingId = findSignalingId(roomId, participantId);
    if (!signalingId) {
        return false;
    }

    return m_eventRouter->routeEvent(*signalingId, event);
}
