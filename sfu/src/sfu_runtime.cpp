#include "sfu_runtime.h"

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
