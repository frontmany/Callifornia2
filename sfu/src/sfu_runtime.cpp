#include "sfu_runtime.h"

#include <algorithm>
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

    // Track this peer under its signalingId for bulk cleanup on stream cancel.
    if (!signalingId.empty()) {
        m_signalingPeers[signalingId].emplace_back(roomId, participantId);
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

        // Remove from signaling index.
        if (session) {
            const auto& signalingId = session->signalingId();
            auto idxIt = m_signalingPeers.find(signalingId);
            if (idxIt != m_signalingPeers.end()) {
                auto& vec = idxIt->second;
                vec.erase(
                    std::remove_if(vec.begin(), vec.end(),
                                   [&](const auto& p) {
                                       return p.first == roomId &&
                                              p.second == participantId;
                                   }),
                    vec.end());
                if (vec.empty()) {
                    m_signalingPeers.erase(idxIt);
                }
            }
        }
        return removed;
    }
}

int SfuRuntime::removePeersBySignaling(const std::string& signalingId) {
    if (signalingId.empty()) {
        return 0;
    }

    // Snapshot the list while holding the lock, then shut down peer connections
    // outside the lock to avoid deadlocks with PeerSession callbacks.
    std::vector<std::pair<std::string, std::string>> toRemove;
    {
        std::lock_guard lock(m_mutex);
        auto idxIt = m_signalingPeers.find(signalingId);
        if (idxIt == m_signalingPeers.end()) {
            return 0;
        }
        toRemove = std::move(idxIt->second);
        m_signalingPeers.erase(idxIt);
    }

    int removed = 0;
    for (const auto& [roomId, participantId] : toRemove) {
        std::shared_ptr<PeerSession> session;
        {
            std::lock_guard lock(m_mutex);
            auto roomIt = m_rooms.find(roomId);
            if (roomIt == m_rooms.end()) {
                continue;
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
                continue;
            }
            if (roomIt->second->removePeer(participantId)) {
                ++removed;
            }
            if (roomIt->second->peerCount() == 0) {
                m_rooms.erase(roomIt);
            }
        }
    }
    return removed;
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
