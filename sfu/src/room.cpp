#include "room.h"

#include "absl/log/log.h"
#include <algorithm>

bool Room::addPeer(const std::shared_ptr<PeerSession>& session) {
    if (!session) {
        return false;
    }
    std::vector<std::string> existingParticipants;
    std::weak_ptr<Room> weakRoom = weak_from_this();
    std::lock_guard lock(m_mutex);
    auto [it, inserted] = m_peers.emplace(session->participantId(), session);
    (void)it;
    if (!inserted) {
        return false;
    }
    existingParticipants.reserve(m_peers.size());
    for (const auto& [participantId, _] : m_peers) {
        if (participantId != session->participantId()) {
            existingParticipants.push_back(participantId);
        }
    }
    for (const auto& participantId : existingParticipants) {
        m_subscribersByPublisher[participantId].insert(session->participantId());
        m_subscribersByPublisher[session->participantId()].insert(participantId);
    }
    session->setTrackHandlers(
        [weakRoom](const std::string& publisherId,
                   const std::shared_ptr<rtc::Track>& track,
                   const std::string& trackId,
                   const std::string& kind,
                   const std::string& sourceType) {
            if (auto room = weakRoom.lock()) {
                room->onPublisherTrackAdded(publisherId, track, trackId, kind, sourceType);
            }
        },
        [weakRoom](const std::string& publisherId,
                   const std::string& trackId,
                   const std::string& kind,
                   const std::string& reason) {
            if (auto room = weakRoom.lock()) {
                room->onPublisherTrackRemoved(publisherId, trackId, kind, reason);
            }
        });
    return true;
}

bool Room::removePeer(const std::string& participantId) {
    std::vector<std::shared_ptr<PeerSession>> peersSnapshot;
    std::vector<std::string> toRemoveTrackIds;
    {
        std::lock_guard lock(m_mutex);
        if (m_peers.erase(participantId) == 0) {
            return false;
        }
        m_subscribersByPublisher.erase(participantId);
        for (auto& [publisher, subscribers] : m_subscribersByPublisher) {
            subscribers.erase(participantId);
            (void)publisher;
        }
        for (const auto& [trackId, _] : m_forwardByTrack) {
            if (trackId.rfind(participantId + "|", 0) == 0) {
                toRemoveTrackIds.push_back(trackId);
            }
        }
        for (const auto& id : toRemoveTrackIds) {
            m_forwardByTrack.erase(id);
        }
        peersSnapshot.reserve(m_peers.size());
        for (const auto& [_, peer] : m_peers) {
            peersSnapshot.push_back(peer);
        }
    }
    for (const auto& relayTrackId : toRemoveTrackIds) {
        for (const auto& peer : peersSnapshot) {
            peer->removeRelayTrack(relayTrackId);
        }
    }
    return true;
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

std::string Room::buildRelayTrackId(const std::string& publisherId, const std::string& sourceTrackId) {
    return publisherId + "|" + sourceTrackId;
}

void Room::onPublisherTrackAdded(const std::string& publisherId,
                                 const std::shared_ptr<rtc::Track>& track,
                                 const std::string& trackId,
                                 const std::string& kind,
                                 const std::string& sourceType) {
    (void)kind;
    (void)sourceType;
    std::vector<std::shared_ptr<PeerSession>> subscribers;
    std::shared_ptr<ForwardEntry> entry;
    const std::string relayTrackId = buildRelayTrackId(publisherId, trackId);
    {
        std::lock_guard lock(m_mutex);
        auto sit = m_subscribersByPublisher.find(publisherId);
        if (sit != m_subscribersByPublisher.end()) {
            subscribers.reserve(sit->second.size());
            for (const auto& subscriberId : sit->second) {
                auto pit = m_peers.find(subscriberId);
                if (pit != m_peers.end()) {
                    subscribers.push_back(pit->second);
                }
            }
        }
        auto [it, inserted] = m_forwardByTrack.emplace(relayTrackId, std::make_shared<ForwardEntry>());
        entry = it->second;
        if (inserted) {
            entry->sourceTrack = track;
        }
    }

    if (!entry) {
        return;
    }
    for (const auto& subscriber : subscribers) {
        std::string err;
        auto relay = subscriber->addRelayTrackIfMissing(relayTrackId, track->description(), err);
        if (!relay) {
            ABSL_LOG(WARNING) << "Failed to create relay track for subscriber participant_id="
                              << subscriber->participantId()
                              << " publisher_id="
                              << publisherId
                              << " track_id="
                              << trackId
                              << " error="
                              << err;
            continue;
        }
        std::lock_guard entryLock(entry->mutex);
        entry->relayTracks.push_back(relay);
    }

    track->onMessage([entryWeak = std::weak_ptr<ForwardEntry>(entry)](auto message) {
        auto entryLocked = entryWeak.lock();
        if (!entryLocked) {
            return;
        }
        std::lock_guard lock(entryLocked->mutex);
        auto& relays = entryLocked->relayTracks;
        relays.erase(
            std::remove_if(relays.begin(), relays.end(), [](const std::weak_ptr<rtc::Track>& weakRelay) {
                return weakRelay.expired();
            }),
            relays.end());
        for (const auto& weakRelay : relays) {
            if (auto relay = weakRelay.lock()) {
                try {
                    relay->send(message);
                } catch (...) {
                }
            }
        }
    });
}

void Room::onPublisherTrackRemoved(const std::string& publisherId,
                                   const std::string& trackId,
                                   const std::string& kind,
                                   const std::string& reason) {
    (void)kind;
    (void)reason;
    std::vector<std::shared_ptr<PeerSession>> subscribers;
    const std::string relayTrackId = buildRelayTrackId(publisherId, trackId);
    {
        std::lock_guard lock(m_mutex);
        auto sit = m_subscribersByPublisher.find(publisherId);
        if (sit != m_subscribersByPublisher.end()) {
            subscribers.reserve(sit->second.size());
            for (const auto& subscriberId : sit->second) {
                auto pit = m_peers.find(subscriberId);
                if (pit != m_peers.end()) {
                    subscribers.push_back(pit->second);
                }
            }
        }
        m_forwardByTrack.erase(relayTrackId);
    }
    for (const auto& subscriber : subscribers) {
        subscriber->removeRelayTrack(relayTrackId);
    }
}
