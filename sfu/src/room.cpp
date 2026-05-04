#include "room.h"

bool Room::addPeer(const std::shared_ptr<PeerSession>& session) {
    if (!session) {
        return false;
    }
    std::lock_guard lock(m_mutex);
    auto [it, inserted] = m_peers.emplace(session->participantId(), session);
    (void)it;
    return inserted;
}

bool Room::removePeer(const std::string& participantId) {
    std::lock_guard lock(m_mutex);
    return m_peers.erase(participantId) > 0;
}

std::shared_ptr<PeerSession> Room::findPeer(const std::string& participantId) const {
    std::lock_guard lock(m_mutex);
    auto it = m_peers.find(participantId);
    if (it == m_peers.end()) {
        return nullptr;
    }
    return it->second;
}

std::size_t Room::peerCount() const {
    std::lock_guard lock(m_mutex);
    return m_peers.size();
}
