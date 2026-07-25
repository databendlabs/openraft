(ns jepsen.openraft.cluster-test
  (:require [clojure.test :refer [deftest is]]
            [jepsen.openraft.client :as client]
            [jepsen.openraft.cluster :as cluster]))

(def test-config
  {:nodes ["n1" "n2" "n3"]
   :api-port 21001})

(deftest finds-the-leader-agreed-on-by-all-nodes
  (let [metrics {"n1:21001" {:state "Follower"
                              :current_leader 2}
                 "n2:21001" {:state "Leader"
                              :current_leader 2}
                 "n3:21001" {:state "Follower"
                              :current_leader 2}}]
    (with-redefs [client/metrics! metrics]
      (let [status (#'cluster/cluster-status test-config)]
        (is (= "n2" (:leader status)))
        (is (= 3 (count (:metrics status))))))))

(deftest rejects-disagreement-about-the-leader
  (let [metrics {"n1:21001" {:state "Leader"
                              :current_leader 1}
                 "n2:21001" {:state "Follower"
                              :current_leader 1}
                 "n3:21001" {:state "Leader"
                              :current_leader 3}}]
    (with-redefs [client/metrics! metrics]
      (is (nil? (#'cluster/cluster-status test-config))))))

(deftest rejects-a-node-without-a-known-leader
  (let [metrics {"n1:21001" {:state "Follower"
                              :current_leader 2}
                 "n2:21001" {:state "Leader"
                              :current_leader 2}
                 "n3:21001" {:state "Follower"
                              :current_leader nil}}]
    (with-redefs [client/metrics! metrics]
      (is (nil? (#'cluster/cluster-status test-config))))))

(defn- membership [configs]
  {:membership_config
   {:membership
    {:configs configs}}})

(deftest maps-voter-configs-to-jepsen-nodes
  (let [status {:leader "n2"
                :metrics {"n2" (membership [[1 2 3]])}}]
    (is (= [#{"n1" "n2" "n3"}]
           (cluster/voter-configs test-config status)))))

(deftest maps-joint-voter-configs
  (let [test (assoc test-config :nodes ["n1" "n2" "n3" "n4"])
        status {:leader "n2"
                :metrics {"n2" (membership [[1 2 3] [1 2 4]])}}]
    (is (= [#{"n1" "n2" "n3"}
            #{"n1" "n2" "n4"}]
           (cluster/voter-configs test status)))))
