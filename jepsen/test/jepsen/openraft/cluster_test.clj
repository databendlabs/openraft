(ns jepsen.openraft.cluster-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.openraft.client :as client]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.util :as util]))

(def test-config
  {:nodes ["n1" "n2" "n3"]
   :api-port 21001})

(defn- spin-millis
  "Burns the calling thread for `ms`, which an interrupt cannot cut short."
  [ms]
  (let [deadline (+ (System/nanoTime) (* ms 1000000))]
    (while (< (System/nanoTime) deadline)
      nil)))

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

(deftest probes-every-node-concurrently
  (let [nodes (:nodes test-config)
        follower {:state "Follower"
                  :current_leader "n2"
                  :vote (vote 3 "n2")}
        arrived (java.util.concurrent.CountDownLatch. (count nodes))]
    (with-redefs [client/metrics!
                  (fn [endpoint]
                    (.countDown arrived)
                    ;; A sequential scan never lets the last node arrive, so
                    ;; the first probe waits out the timeout and fails here.
                    (when-not (.await arrived
                                      10
                                      java.util.concurrent.TimeUnit/SECONDS)
                      (throw (ex-info "a node probe waited for an earlier one"
                                      {:endpoint endpoint})))
                    follower)]
      (is (= (zipmap nodes (repeat follower))
             (#'cluster/collect-reachable-metrics test-config))))))

(deftest restores-an-interrupt-after-draining-every-probe
  (let [node-count (count (:nodes test-config))
        probing (java.util.concurrent.CountDownLatch. node-count)
        cleaned (java.util.concurrent.CountDownLatch. node-count)
        coordinator (promise)
        scan (future
               (deliver coordinator (Thread/currentThread))
               (try
                 (with-redefs [client/metrics!
                               (fn [_endpoint]
                                 (.countDown probing)
                                 (try
                                   (Thread/sleep 60000)
                                   nil
                                   (catch InterruptedException e
                                     ;; A cancelled probe still runs cleanup,
                                     ;; and the scan owes it that time.
                                     (spin-millis 150)
                                     (.countDown cleaned)
                                     (throw e))))]
                   (#'cluster/collect-reachable-metrics test-config)
                   [nil
                    (.isInterrupted (Thread/currentThread))
                    (.getCount cleaned)])
                 (catch Exception e
                   [e
                    (.isInterrupted (Thread/currentThread))
                    (.getCount cleaned)])
                 (finally
                   (Thread/interrupted))))]
    (is (.await probing 10 java.util.concurrent.TimeUnit/SECONDS))
    (.interrupt ^Thread @coordinator)
    (let [outcome (deref scan 30000 ::timeout)]
      (is (vector? outcome) "the scan never returned")
      (when (vector? outcome)
        (let [[thrown interrupted? unfinished] outcome]
          (is (instance? InterruptedException thrown))
          (is interrupted?)
          (is (zero? unfinished)))))))

(deftest distinguishes-modeled-metrics-failures-from-harness-errors
  (testing "recognized SUT observations make a node unavailable"
    (doseq [kind [:http-error
                  :invalid-json
                  :invalid-response
                  :openraft-error
                  :request-timeout
                  :transport-error
                  :unreachable]]
      (with-redefs [client/metrics!
                    (fn [_endpoint]
                      (throw (ex-info "unavailable" {:kind kind})))]
        (is (nil? (#'cluster/cluster-status test-config)) (name kind)))))

  (testing "unknown implementation exceptions propagate"
    (let [error (RuntimeException. "metrics bug")
          thrown (with-redefs [client/metrics! (fn [_endpoint]
                                                 (throw error))]
                   (try
                     (#'cluster/cluster-status test-config)
                     nil
                     (catch Exception e
                       e)))]
      (is (identical? error thrown))))

  (testing "the readiness wait preserves the unknown exception"
    (let [error (RuntimeException. "readiness bug")
          thrown (with-redefs [client/metrics! (fn [_endpoint]
                                                 (throw error))]
                   (try
                     (cluster/await-ready! test-config)
                     nil
                     (catch Exception e
                       e)))]
      (is (identical? error thrown)))))

(deftest preserves-metrics-request-interruption
  (doseq [[label make-error]
          [[:interrupted-exception
            #(InterruptedException. "interrupted")]
           [:interrupted-io
            #(java.io.InterruptedIOException. "interrupted")]
           [:closed-by-interrupt
            #(java.nio.channels.ClosedByInterruptException.)]
           [:wrapped
            #(ex-info "interrupted" {:kind :interrupted})]]]
    (let [error (make-error)
          [thrown interrupted?]
          @(future
             (try
               (with-redefs [client/metrics!
                             (fn [_endpoint]
                               (throw error))]
                 (#'cluster/cluster-status test-config)
                 [nil (.isInterrupted (Thread/currentThread))])
               (catch Exception e
                 [e (.isInterrupted (Thread/currentThread))])
               (finally
                 (Thread/interrupted))))]
      (is (identical? error thrown) (name label))
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

(defn- stable-status [leader metrics voters]
  {:leader leader
   :metrics metrics
   :effective-log-id {:index 7}
   :committed-log-id {:index 7}
   :effective-voter-configs [voters]
   :committed-voter-configs [voters]
   :voters voters
   :learners #{}
   :non-members #{}
   :stable? true})

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

(deftest refreshes-reachable-metrics-after-an-election
  (let [membership (stored-membership
                    7
                    [["n1" "n2" "n3" "n4" "n5"]]
                    ["n1" "n2" "n3" "n4" "n5"])
        leader-metrics (metrics "Leader" 18 "n1"
                                membership membership)
        follower-metrics (metrics "Follower" 18 "n1"
                                  membership membership)
        current {"n1:21001" leader-metrics
                 "n4:21001" follower-metrics
                 "n5:21001" follower-metrics}
        stale (metrics "Follower" 17 "n3" membership membership)
        voters #{"n1" "n2" "n3" "n4" "n5"}
        attempts (atom {})]
    (with-redefs [client/metrics!
                  (fn [endpoint]
                    (let [counts (swap! attempts update endpoint
                                        (fnil inc 0))]
                      (cond
                        (#{"n2:21001" "n3:21001"} endpoint)
                        (throw (ex-info "unreachable" {:kind :unreachable}))

                        (and (= "n1:21001" endpoint)
                             (= 1 (get counts endpoint)))
                        stale

                        :else
                        (get current endpoint))))]
      (is (= (stable-status "n1"
                            {"n1" leader-metrics
                             "n4" follower-metrics
                             "n5" follower-metrics}
                            voters)
             (cluster/membership-status five-node-config)))
      (is (= {"n1:21001" 2
              "n2:21001" 1
              "n3:21001" 1
              "n4:21001" 2
              "n5:21001" 2}
             @attempts)))))

(deftest refreshes-a-stale-quorum-behind-a-newer-committed-vote
  (let [membership (stored-membership
                    7
                    [["n1" "n2" "n3" "n4" "n5"]]
                    ["n1" "n2" "n3" "n4" "n5"])
        old-leader (metrics "Leader" 17 "n1" membership membership)
        old-follower (metrics "Follower" 17 "n1" membership membership)
        new-leader (metrics "Leader" 18 "n4" membership membership)
        new-follower (metrics "Follower" 18 "n4" membership membership)
        first-scan {"n1:21001" old-leader
                    "n2:21001" old-follower
                    "n3:21001" old-follower
                    "n4:21001" new-leader
                    "n5:21001" new-follower}
        current-scan {"n1:21001" new-follower
                      "n2:21001" new-follower
                      "n3:21001" new-follower
                      "n4:21001" new-leader
                      "n5:21001" new-follower}
        voters #{"n1" "n2" "n3" "n4" "n5"}
        attempts (atom {})]
    (with-redefs [client/metrics!
                  (fn [endpoint]
                    (let [counts (swap! attempts update endpoint
                                        (fnil inc 0))]
                      (get (if (= 1 (get counts endpoint))
                             first-scan
                             current-scan)
                           endpoint)))]
      (is (= (stable-status "n4"
                            {"n1" new-follower
                             "n2" new-follower
                             "n3" new-follower
                             "n4" new-leader
                             "n5" new-follower}
                            voters)
             (cluster/membership-status five-node-config)))
      (is (= {"n1:21001" 2
              "n2:21001" 2
              "n3:21001" 2
              "n4:21001" 2
              "n5:21001" 2}
             @attempts)))))

(deftest accepts-a-supported-leader-despite-a-newer-uncommitted-vote
  (let [membership (stored-membership
                    7
                    [["n1" "n2" "n3"]]
                    ["n1" "n2" "n3"])
        leader-metrics (metrics "Leader" 17 "n1"
                                membership membership)
        follower-metrics (metrics "Follower" 17 "n1"
                                  membership membership)
        candidate-metrics (-> (metrics "Candidate" 18 "n3"
                                       membership membership)
                              (assoc-in [:vote :committed] false))
        observed {"n1" leader-metrics
                  "n2" follower-metrics
                  "n3" candidate-metrics}]
    (is (= ["n1" leader-metrics]
           (#'cluster/supported-leader observed)))))

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

(deftest membership-waits-do-not-retry-harness-exceptions
  (doseq [[label wait!]
          [[:committed #(cluster/await-committed-membership! test-config)]
           [:stable #(cluster/await-stable-membership! test-config)]
           [:learner #(cluster/await-observed-learner!
                       test-config
                       "n4"
                       100)]]]
    (testing (name label)
      (let [attempts (atom 0)
            error (RuntimeException. "membership wait bug")
            thrown (with-redefs [cluster/membership-status
                                 (fn [_test]
                                   (swap! attempts inc)
                                   (throw error))]
                     (try
                       (wait!)
                       nil
                       (catch Exception e
                         e)))]
        (is (identical? error thrown))
        (is (= 1 @attempts)))))

  (testing "node metrics retries only modeled SUT availability"
    (let [attempts (atom 0)
          error (RuntimeException. "node metrics bug")
          thrown (with-redefs [cluster/node-metrics!
                               (fn [_test _node]
                                 (swap! attempts inc)
                                 (throw error))]
                   (try
                     (cluster/await-node-metrics! test-config "n4" 100)
                     nil
                     (catch Exception e
                       e)))]
      (is (identical? error thrown))
      (is (= 1 @attempts)))))

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
