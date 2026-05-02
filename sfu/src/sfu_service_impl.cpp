#include "sfu_service_impl.h"

#include <thread>

// #include <rtc/rtc.hpp>  // next step: map peers to rtc::PeerConnection

#include "absl/log/log.h"

grpc::Status SfuServiceImpl::CreatePeer(grpc::ServerContext* context,
                                        const sfu::CreatePeerRequest* request,
                                        sfu::CreatePeerResponse* response) {
    (void)context;
    ABSL_LOG(INFO) << "CreatePeer peer_id=" << request->peer_id()
                   << " room_id=" << request->room_id()
                   << " signaling_id=" << request->signaling_id();
    // TODO: construct rtc::PeerConnection / room state; no SDP yet.
    response->set_success(true);
    return grpc::Status::OK;
}

grpc::Status SfuServiceImpl::DeletePeer(grpc::ServerContext* context,
                                        const sfu::DeletePeerRequest* request,
                                        sfu::DeletePeerResponse* response) {
    (void)context;
    ABSL_LOG(INFO) << "DeletePeer peer_id=" << request->peer_id()
                   << " reason=" << request->reason();
    // TODO: tear down PeerConnection and tracks.
    response->set_success(true);
    return grpc::Status::OK;
}

grpc::Status SfuServiceImpl::HandleSDP(grpc::ServerContext* context,
                                       const sfu::HandleSDPRequest* request,
                                       sfu::HandleSDPResponse* response) {
    (void)context;
    ABSL_LOG(INFO) << "HandleSDP peer_id=" << request->peer_id()
                   << " type=" << static_cast<int>(request->type())
                   << " sdp_bytes=" << request->sdp().size();
    // TODO: setRemoteDescription / createAnswer or handle renegotiation.
    response->set_success(false);
    response->set_error_message("HandleSDP not wired to libdatachannel yet");
    response->set_type(sfu::SdpType::SDP_TYPE_UNSPECIFIED);
    return grpc::Status::OK;
}

grpc::Status SfuServiceImpl::AddICECandidate(grpc::ServerContext* context,
                                             const sfu::AddICECandidateRequest* request,
                                             sfu::AddICECandidateResponse* response) {
    (void)context;
    ABSL_LOG(INFO) << "AddICECandidate peer_id=" << request->peer_id();
    // TODO: pc.addRemoteCandidate(...)
    response->set_success(false);
    response->set_error_message("ICE not wired to libdatachannel yet");
    return grpc::Status::OK;
}

grpc::Status SfuServiceImpl::SubscribeEvents(grpc::ServerContext* context,
                                             const sfu::SubscribeEventsRequest* request,
                                             grpc::ServerWriter<sfu::SFUEvent>* writer) {
    ABSL_LOG(INFO) << "SubscribeEvents signaling_id=" << request->signaling_id();
    // Signaling keeps this stream open; closing it triggers reconnect and room teardown.
    while (!context->IsCancelled()) {
        sfu::SFUEvent ev;
        ev.set_peer_id("");
        ev.set_room_id("");
        auto* hb = ev.mutable_heartbeat();
        hb->set_timestamp(
            std::chrono::duration_cast<std::chrono::seconds>(
                std::chrono::system_clock::now().time_since_epoch())
                .count());
        hb->set_active_peers(0);
        hb->set_active_rooms(0);
        if (!writer->Write(ev)) {
            break;
        }
        std::this_thread::sleep_for(kHeartbeatInterval);
    }
    return grpc::Status::OK;
}

grpc::Status SfuServiceImpl::Ping(grpc::ServerContext* context,
                                  const sfu::PingRequest* request,
                                  sfu::PingResponse* response) {
    (void)context;
    (void)request;
    response->set_ok(true);
    return grpc::Status::OK;
}
