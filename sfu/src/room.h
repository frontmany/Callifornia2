#pragma once

#include "peer_session.h"

#include <cstddef>
#include <memory>
#include <mutex>
#include <string>
#include <unordered_set>
#include <unordered_map>
#include <vector>

class Room : public std::enable_shared_from_this<Room> {
public:
    explicit Room(std::string roomId) : m_roomId(std::move(roomId)) {}

    const std::string& roomId() const { return m_roomId; }

    bool addPeer(const std::shared_ptr<PeerSession>& session);
    bool removePeer(const std::string& participantId);
    std::shared_ptr<PeerSession> findPeer(const std::string& participantId) const;

    std::size_t peerCount() const;

private:
    struct ForwardEntry {
        std::shared_ptr<rtc::Track> sourceTrack;
        std::vector<std::weak_ptr<rtc::Track>> relayTracks;
        std::mutex mutex;
    };

    static std::string buildRelayTrackId(const std::string& publisherId, const std::string& sourceTrackId);
    void onPublisherTrackAdded(const std::string& publisherId,
                               const std::shared_ptr<rtc::Track>& track,
                               const std::string& trackId,
                               const std::string& kind,
                               const std::string& sourceType);
    void onPublisherTrackRemoved(const std::string& publisherId,
                                 const std::string& trackId,
                                 const std::string& kind,
                                 const std::string& reason);

    std::string m_roomId;
    mutable std::mutex m_mutex;
    std::unordered_map<std::string, std::shared_ptr<PeerSession>> m_peers;
    std::unordered_map<std::string, std::unordered_set<std::string>> m_subscribersByPublisher;
    std::unordered_map<std::string, std::shared_ptr<ForwardEntry>> m_forwardByTrack;
};
