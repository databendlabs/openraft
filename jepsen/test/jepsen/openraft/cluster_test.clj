(ns jepsen.openraft.cluster-test
  (:require [clojure.test :refer [deftest is]]
            [jepsen.openraft.client :as client]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.util :as util]))

(def test-config
  {:nodes ["n1" "n2" "n3"]
   :api-port 21001})

(defn- vote [term leader]
  {:leader_id {:term term
               :node_id leader}
   :committed true})

(deftest finds-the-leader-agreed-on-by-all-nodes
  (let [metrics {"n1:21001" {:state "Follower"
                              :current_leader "n2"
                              :vote (vote 3 "n2")}
                 "n2:21001" {:state "Leader"
                              :current_leader "n2"
                              :vote (vote 3 "n2")}
                 "n3:21001" {:state "Follower"
                              :current_leader "n2"
                              :vote (vote 3 "n2")}}]
    (with-redefs [client/metrics! metrics]
      (let [status (#'cluster/cluster-status test-config)]
        (is (= "n2" (:leader status)))
        (is (= 3 (count (:metrics status))))))))

(deftest rejects-disagreement-about-the-leader
  (let [metrics {"n1:21001" {:state "Leader"
                              :current_leader "n1"
                              :vote (vote 2 "n1")}
                 "n2:21001" {:state "Follower"
                              :current_leader "n1"
                              :vote (vote 2 "n1")}
                 "n3:21001" {:state "Leader"
                              :current_leader "n3"
                              :vote (vote 3 "n3")}}]
    (with-redefs [client/metrics! metrics]
      (is (nil? (#'cluster/cluster-status test-config))))))

(deftest rejects-a-node-without-a-known-leader
  (let [metrics {"n1:21001" {:state "Follower"
                              :current_leader "n2"
                              :vote (vote 3 "n2")}
                 "n2:21001" {:state "Leader"
                              :current_leader "n2"
                              :vote (vote 3 "n2")}
                 "n3:21001" {:state "Follower"
                              :current_leader nil
                              :vote (vote 3 "n2")}}]
    (with-redefs [client/metrics! metrics]
      (is (nil? (#'cluster/cluster-status test-config))))))

(deftest rejects-an-unreachable-test-node
  (let [metrics {"n1:21001" {:state "Follower"
                              :current_leader "n2"
                              :vote (vote 3 "n2")}
                 "n2:21001" {:state "Leader"
                              :current_leader "n2"
                              :vote (vote 3 "n2")}}]
    (with-redefs [client/metrics!
                  (fn [endpoint]
                    (or (get metrics endpoint)
                        (throw (ex-info "unreachable"
                                        {:kind :unreachable}))))]
      (is (nil? (#'cluster/cluster-status test-config))))))

(deftest preserves-metrics-request-interruption
  (doseq [[label error] [[:raw #(InterruptedException. "interrupted")]
                         [:wrapped #(ex-info "interrupted"
                                            {:kind :interrupted})]]]
    (let [interrupted?
          @(future
             (with-redefs [client/metrics!
                           (fn [_endpoint]
                             (throw (error)))]
               (try
                 (#'cluster/cluster-status test-config)
                 false
                 (catch InterruptedException _
                   (let [interrupted? (.isInterrupted
                                       (Thread/currentThread))]
                     (Thread/interrupted)
                     interrupted?)))))]
      (is interrupted? (name label)))))

(defn- membership [configs]
  {:membership_config
   {:membership
    {:configs configs}}})

(defn- stored-membership [index configs nodes]
  {:log_id {:index index}
   :membership
   {:configs configs
    :nodes (into {}
                 (map (fn [node-id]
                        [(keyword (str node-id)) {}])
                      nodes))}})

(defn- metrics
  ([state leader effective committed]
   (metrics state 1 leader effective committed))
  ([state term leader effective committed]
   {:state state
    :current_leader leader
    :vote (when leader (vote term leader))
    :membership_config effective
    :committed_membership_config committed}))

(def five-node-config
  {:nodes ["n1" "n2" "n3" "n4" "n5"]
   :api-port 21001})

(deftest observes-a-stable-membership
  (let [membership (stored-membership
                     7
                     [["n1" "n2" "n3"]]
                     ["n1" "n2" "n3" "n4"])
        responses {"n1:21001" (metrics "Leader" "n1"
                                       membership membership)
                   "n2:21001" (metrics "Follower" "n1"
                                       membership membership)
                   "n3:21001" (metrics "Follower" "n1"
                                       membership membership)
                   "n4:21001" (metrics "Learner" "n1"
                                       membership membership)}]
    (with-redefs [client/metrics!
                  (fn [endpoint]
                    (or (get responses endpoint)
                        (throw (ex-info "unreachable"
                                        {:kind :unreachable}))))]
      (let [status (cluster/membership-status five-node-config)]
        (is (:stable? status))
        (is (= "n1" (:leader status)))
        (is (= [#{"n1" "n2" "n3"}]
               (:effective-voter-configs status)))
        (is (= #{"n1" "n2" "n3"} (:voters status)))
        (is (= #{"n4"} (:learners status)))
        (is (= #{"n5"} (:non-members status)))))))

(deftest observes-a-joint-membership
  (let [committed (stored-membership
                    7
                    [["n1" "n2" "n3"]]
                    ["n1" "n2" "n3" "n4"])
        effective (stored-membership
                    8
                    [["n1" "n2" "n3"]
                     ["n1" "n2" "n3" "n4"]]
                    ["n1" "n2" "n3" "n4"])
        responses {"n1:21001" (metrics "Leader" "n1"
                                       effective committed)
                   "n2:21001" (metrics "Follower" "n1"
                                       effective committed)
                   "n3:21001" (metrics "Follower" "n1"
                                       effective committed)
                   "n4:21001" (metrics "Follower" "n1"
                                       effective committed)
                   "n5:21001" (metrics "Learner" nil effective committed)}]
    (with-redefs [client/metrics! responses]
      (let [status (cluster/membership-status five-node-config)]
        (is (false? (:stable? status)))
        (is (= [#{"n1" "n2" "n3"}
                #{"n1" "n2" "n3" "n4"}]
               (:effective-voter-configs status)))
        (is (= [#{"n1" "n2" "n3"}]
               (:committed-voter-configs status)))
        (is (= #{"n1" "n2" "n3" "n4"} (:voters status)))
        (is (empty? (:learners status)))
        (is (= #{"n5"} (:non-members status)))))))

(deftest waits-until-the-effective-membership-is-committed
  (let [uncommitted {:effective-log-id {:index 8}
                     :committed-log-id {:index 7}}
        committed {:effective-log-id {:index 8}
                   :committed-log-id {:index 8}}
        status (atom uncommitted)
        attempts (atom [])]
    (with-redefs [cluster/membership-status (fn [_test] @status)
                  util/await-fn
                  (fn [f _opts]
                    (swap! attempts conj
                           (try
                             (f)
                             :returned
                             (catch Exception _
                               :retry)))
                    (reset! status committed)
                    (f))]
      (is (= committed
             (cluster/await-committed-membership! test-config)))
      (is (= [:retry] @attempts)))))

(deftest rejects-support-from-an-older-leader-term
  (let [membership (stored-membership
                     7
                     [["n1" "n2" "n3"]]
                     ["n1" "n2" "n3"])
        responses {"n1:21001" (metrics "Leader" 3 "n1"
                                       membership membership)
                   "n2:21001" (metrics "Follower" 2 "n1"
                                       membership membership)
                   "n3:21001" (metrics "Follower" 2 "n1"
                                       membership membership)}]
    (with-redefs [client/metrics! responses]
      (is (nil? (cluster/membership-status test-config))))))

(deftest ignores-a-stale-removed-leader
  (let [old-membership (stored-membership
                         7
                         [["n1" "n2" "n3" "n4" "n5"]]
                         ["n1" "n2" "n3" "n4" "n5"])
        membership (stored-membership
                     9
                     [["n2" "n3" "n4"]]
                     ["n2" "n3" "n4"])
        responses {"n1:21001" (metrics "Leader" "n1"
                                       old-membership old-membership)
                   "n2:21001" (metrics "Leader" "n2"
                                       membership membership)
                   "n3:21001" (metrics "Follower" "n2"
                                       membership membership)
                   "n4:21001" (metrics "Follower" "n2"
                                       membership membership)
                   "n5:21001" (metrics "Candidate" "n1"
                                       old-membership old-membership)}]
    (with-redefs [client/metrics! responses]
      (let [status (cluster/membership-status five-node-config)]
        (is (= "n2" (:leader status)))
        (is (= #{"n2" "n3" "n4"} (:voters status)))
        (is (= #{"n1" "n5"} (:non-members status)))))))

(deftest maps-joint-voter-configs
  (let [test (assoc test-config
                    :nodes ["n1" "n2" "n3" "n4"])
        status {:leader "n2"
                :metrics {"n2" (membership [["n1" "n2" "n3"]
                                            ["n1" "n2" "n4"]])}}]
    (is (= [#{"n1" "n2" "n3"}
            #{"n1" "n2" "n4"}]
           (cluster/voter-configs test status)))))
