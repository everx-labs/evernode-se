use super::*;

/// Routing information about node
#[derive(Debug, Clone, Eq, PartialEq, Default, Hash)]
pub struct NodeInfo {
    validator_index: usize,
    shard: ShardIdent
}

impl NodeInfo {
    pub fn new(index: usize, shard: ShardIdent) -> Self {
        Self {
            validator_index: index,
            shard
        }
    }
    pub fn is_same_shard(&self, shard: &ShardIdent) -> bool {
        &self.shard == shard
    }
}

// key for workchain - validatior => shardes
#[derive(Debug, Clone, Eq, PartialEq, Default, Hash)]
struct WcVal {
    wc: i32,
    val: usize
}

#[derive(Debug, Clone, Eq, PartialEq, Default, Hash)]
struct ShardPeer {
    shard: ShardIdent,
    peer: usize,
}

/// struct for storing routing info
#[derive(Default)]
pub struct RoutingTable {
    _shard: ShardIdent,
    route_by_info: Arc<Mutex<HashMap<NodeInfo, usize>>>,
    route_by_peer: Arc<Mutex<HashMap<usize, NodeInfo>>>,
    route_by_wcval: Arc<Mutex<HashMap<WcVal, Vec<ShardPeer>>>>,
}


impl RoutingTable {

    /// Create new instance of RoutingTable
    pub fn new(_shard: ShardIdent) -> Self {
        Self {
            _shard,
            route_by_info: Arc::new(Mutex::new(HashMap::new())),
            route_by_peer: Arc::new(Mutex::new(HashMap::new())),
            route_by_wcval: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Insert new item to table
    pub fn insert(&self, info: NodeInfo, peer: usize) -> Option<usize> {
        self.route_by_peer.lock().insert(peer, info.clone());
        let wv = WcVal { wc: info.shard.workchain_id(), val: info.validator_index };
        let sp = ShardPeer { shard: info.shard.clone(), peer };
        self.route_by_wcval.lock().entry(wv).or_insert(vec![]).push(sp);
        
        self.route_by_info.lock().insert(info, peer)
    }

    /// Delete record from routing table
    pub fn remove_by_info(&self, info: &NodeInfo) -> Option<usize> {
        let peer = self.route_by_info.lock().remove(&info);
        if let Some(peer) = peer {
            self.route_by_peer.lock().remove(&peer);
            self.remove_from_wcval(&info, peer);
        }   
        peer
    }

    fn remove_from_wcval(&self, info: &NodeInfo, peer: usize) {
        let wv = WcVal { wc: info.shard.workchain_id(), val: info.validator_index };
        let sp = ShardPeer { shard: info.shard.clone(), peer };
        let index = self
            .route_by_wcval
            .lock()
            .entry(wv.clone())
            .or_insert(vec![])
            .iter()
            .position(|x| *x == sp.clone());

        if let Some(index) = index {
            self.route_by_wcval
                .lock()
                .entry(wv)
                .or_insert(vec![])
                .remove(index);
        }
    }

    /// Remove record by peed id
    pub fn remove_by_peer(&self, peer: &usize) -> Option<NodeInfo> {
        let info = self.route_by_peer.lock().remove(&peer);
        if let Some(ref info) = info {
            self.route_by_info.lock().remove(info);
            self.remove_from_wcval(&info, peer.clone());
        }
        info
    }

    /// Get peer_id for specified parameters
    pub fn get_route_by_info(&self, info: &NodeInfo) -> Option<usize> {
        self.route_by_info.lock().get(info).map(|r| r.clone())
    }

    pub fn get_info_by_peer(&self, peer: &usize) -> Option<NodeInfo> {
        self.route_by_peer.lock().get(peer).map(|r| r.clone())
    }

    pub fn get_route_by_wc_addr_and_validator(&self, wc: i32, addr: AccountId, val_idx: usize) -> Option<usize> {
        let wv = WcVal { wc, val: val_idx };
        if let Some(vec) = self.route_by_wcval.lock().get(&wv) {
            for sp in vec.iter() {
                if messages::is_in_current_shard(&sp.shard, wc, &addr) {
                    return Some(sp.peer);
                }
            }
            return None;
        }
        return None;
    }

}
