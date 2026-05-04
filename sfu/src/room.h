#pragma once

#include "peer_session.h"

#include <cstddef>
#include <memory>
#include <mutex>
#include <string>
#include <unordered_map>

class Room {
public:
    explicit Room(std::string roomId) : m_roomId(std::move(roomId)) {}

    const std::string& roomId() const { return m_roomId; }

    bool addPeer(const std::shared_ptr<PeerSession>& session);
    bool removePeer(const std::string& participantId);
    std::shared_ptr<PeerSession> findPeer(const std::string& participantId) const;

    std::size_t peerCount() const;

private:
    std::string m_roomId;
    mutable std::mutex m_mutex;
    std::unordered_map<std::string, std::shared_ptr<PeerSession>> m_peers;
};
