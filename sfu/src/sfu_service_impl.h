#pragma once

#include <chrono>
#include <grpcpp/grpcpp.h>

#include "signaling.grpc.pb.h"

// Skeleton SFU gRPC service. libdatachannel PeerConnection wiring goes in the .cpp
// (CreatePeer / HandleSDP / ICE / track callbacks) and feeds SFUEvent via a shared hub.

class SfuServiceImpl final : public sfu::SFUService::Service {
public:
    grpc::Status CreatePeer(grpc::ServerContext* context,
                            const sfu::CreatePeerRequest* request,
                            sfu::CreatePeerResponse* response) override;

    grpc::Status DeletePeer(grpc::ServerContext* context,
                            const sfu::DeletePeerRequest* request,
                            sfu::DeletePeerResponse* response) override;

    grpc::Status HandleSDP(grpc::ServerContext* context,
                           const sfu::HandleSDPRequest* request,
                           sfu::HandleSDPResponse* response) override;

    grpc::Status AddICECandidate(grpc::ServerContext* context,
                                 const sfu::AddICECandidateRequest* request,
                                 sfu::AddICECandidateResponse* response) override;

    grpc::Status SubscribeEvents(grpc::ServerContext* context,
                                 const sfu::SubscribeEventsRequest* request,
                                 grpc::ServerWriter<sfu::SFUEvent>* writer) override;

    grpc::Status Ping(grpc::ServerContext* context,
                      const sfu::PingRequest* request,
                      sfu::PingResponse* response) override;

private:
    static constexpr std::chrono::seconds kHeartbeatInterval{15};
};
