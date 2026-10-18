(ns jepsen.openraft.nemesis.snapshot-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
             [nemesis :as nemesis]]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.nemesis.snapshot :as snapshot]
            [jepsen.random :as random]))

(def nodes ["n1" "n2" "n3" "n4" "n5"])

(def test-config
  {:nodes nodes
   :api-port 21001
   :raft-port 22001})

(defn- recording-nemesis
  "Records each invocation and answers it with (respond op)."
  [invocations respond]
  (reify nemesis/Nemesis
    (setup! [this _test]
      this)
    (invoke! [_ _test op]
      (swap! invocations conj (select-keys op [:f :value]))
      (assoc op :value (respond op)))
    (teardown! [this _test]
      this)))

(defn- status [purged-index]
  {:leader "n1"
   :metrics (into {}
                  (map (fn [node]
                         [node {:last_log_index 40
                                :purged (when purged-index
                                          {:index purged-index})}]))
                  nodes)})

(defn- subject [partitioner db-nemesis]
  (snapshot/->SnapshotLagNemesis partitioner db-nemesis (atom nil)))

(deftest isolates-a-follower-until-the-leader-purges-past-it
  (let [partitions (atom [])
        controls (atom [])
        ;; Sampled before the fault the victim ends at 40; by the time the
        ;; fault is in place the leader has reached 44.
        statuses (atom [(status 30)
                        (assoc-in (status 30) [:metrics "n1" :last_log_index] 44)
                        (status 41)
                        (status 45)])
        nemesis (subject (recording-nemesis partitions (constantly nil))
                         (recording-nemesis controls
                                            (fn [_] (throw (AssertionError.
                                                            "isolate must not kill")))))]
    (with-redefs [cluster/membership-status
                  (fn [_test]
                    (let [s (first @statuses)]
                      (when (next @statuses)
                        (swap! statuses rest))
                      s))
                  cluster/voter-configs (fn [_test _status] [(set nodes)])
                  random/shuffle identity
                  snapshot/install-count! (let [counts (atom [0 1])]
                                            (fn [_test node]
                                              (is (= "n2" node))
                                              (let [n (first @counts)]
                                                (swap! counts rest)
                                                n)))
                  cluster/node-metrics!
                  (fn [_test node]
                    (is (= "n2" node))
                    {:last_applied {:index 45}})]
      (let [started (nemesis/invoke! nemesis test-config
                                     {:type :info
                                      :f :start-snapshot-lag
                                      :value :isolate})]
        (is (= :installed (get-in started [:value :status])))
        (is (= "n2" (get-in started [:value :victim])))
        (is (= 40 (get-in started [:value :victim-last-log-index])))
        (is (= 44 (get-in started [:value :log-bound])))
        (is (= [{:f :start
                 :value {"n2" #{"n1" "n3" "n4" "n5"}
                         "n1" #{"n2"}
                         "n3" #{"n2"}
                         "n4" #{"n2"}
                         "n5" #{"n2"}}}]
               @partitions)))

      (let [stopped (nemesis/invoke! nemesis test-config
                                     {:type :info :f :stop-snapshot-lag})
            value (:value stopped)]
        (is (= :installed (:status value)))
        (is (true? (:purged-past-victim? value)))
        (is (= 45 (:purged-index value))
            "a purge to 41 passes the stale sample but not the bound of 44")
        (is (true? (:caught-up? value)))
        (is (= [0 1] [(:installs-before value) (:installs-after value)])
            "the victim's install count is read before and after")
        (is (= {:f :stop :value nil} (last @partitions)))
        (is (empty? @controls))))))

(deftest kills-and-restarts-a-follower
  (let [controls (atom [])
        nemesis (subject (recording-nemesis (atom []) (constantly nil))
                         (recording-nemesis controls
                                            (fn [{:keys [f]}]
                                              {"n2" (case f
                                                      :kill :killed
                                                      :resume :target-absent
                                                      :start :start-confirmed)})))]
    (with-redefs [cluster/membership-status (constantly (status 50))
                  cluster/voter-configs (fn [_test _status] [(set nodes)])
                  random/shuffle identity
                  snapshot/install-count! (constantly 0)
                  cluster/node-metrics! (constantly {:last_applied {:index 50}})]
      (is (= :installed
             (get-in (nemesis/invoke! nemesis test-config
                                      {:type :info
                                       :f :start-snapshot-lag
                                       :value :kill})
                     [:value :status])))
      (let [value (:value (nemesis/invoke! nemesis test-config
                                           {:type :info
                                            :f :stop-snapshot-lag}))]
        (is (= [{:f :kill :value ["n2"]}
                {:f :resume :value ["n2"]}
                {:f :start :value ["n2"]}]
               @controls)
            "a victim another fault paused meanwhile is resumed before the start")
        (is (= :killed (:kill-result value)))
        (is (= :start-confirmed (:start-result value)))
        (is (true? (:caught-up? value)))))))

(deftest gives-up-the-purge-wait-when-commits-stall
  (let [partitions (atom [])
        nemesis (subject (recording-nemesis partitions (constantly nil))
                         (recording-nemesis (atom []) (constantly nil)))
        stalled (assoc-in (status 30) [:metrics "n1" :local_committed] {:index 40})]
    (with-redefs [cluster/membership-status (constantly stalled)
                  cluster/voter-configs (fn [_test _status] [(set nodes)])
                  random/shuffle identity
                  snapshot/install-count! (constantly 0)
                  snapshot/commit-stall-ms 300
                  cluster/node-metrics!
                  (fn [& _] (throw (AssertionError. "no catch-up without a purge")))]
      (nemesis/invoke! nemesis test-config
                       {:type :info :f :start-snapshot-lag :value :isolate})
      (let [started (System/nanoTime)
            value (:value (nemesis/invoke! nemesis test-config
                                           {:type :info :f :stop-snapshot-lag}))]
        (is (< (- (System/nanoTime) started) 5000000000)
            "the wait ends with the stall, well before the purge timeout")
        (is (= :installed (:status value)))
        (is (false? (:purged-past-victim? value)))
        (is (= :no-commit-progress (:purge-wait-gave-up value)))
        (is (not (contains? value :caught-up?)))
        (is (= {:f :stop :value nil} (last @partitions))
            "the follower is still rejoined")))))

(deftest cleanup-rejoins-without-waiting-for-a-purge
  (let [partitions (atom [])
        nemesis (subject (recording-nemesis partitions (constantly nil))
                         (recording-nemesis (atom []) (constantly nil)))]
    (with-redefs [cluster/voter-configs (fn [_test _status] [(set nodes)])
                  random/shuffle identity
                  snapshot/install-count! (constantly 0)]
      (with-redefs [cluster/membership-status (constantly (status nil))]
        (nemesis/invoke! nemesis test-config
                         {:type :info :f :start-snapshot-lag :value :isolate}))
      (with-redefs [cluster/membership-status
                    (fn [_] (throw (AssertionError. "cleanup must not poll")))]
        (let [value (:value (nemesis/invoke! nemesis test-config
                                             {:type :info
                                              :f :stop-snapshot-lag
                                              :value :cleanup}))]
          (is (= :installed (:status value)))
          (is (false? (:purged-past-victim? value)))
          (is (not (contains? value :caught-up?)))
          (is (= {:f :stop :value nil} (last @partitions)))))
      (is (= :skipped
             (get-in (nemesis/invoke! nemesis test-config
                                      {:type :info
                                       :f :stop-snapshot-lag
                                       :value :cleanup})
                     [:value :status]))
          "a second cleanup finds no open episode"))))

(deftest skips-without-a-safe-follower
  (let [nemesis (subject (recording-nemesis (atom []) (constantly nil))
                         (recording-nemesis (atom []) (constantly nil)))]
    (testing "no supported leader"
      (with-redefs [cluster/membership-status (constantly nil)]
        (is (= :no-supported-leader
               (get-in (nemesis/invoke! nemesis test-config
                                        {:type :info
                                         :f :start-snapshot-lag
                                         :value :isolate})
                       [:value :reason])))))

    (testing "voters other faults took down do not count towards the quorum"
      (with-redefs [cluster/membership-status
                    (constantly (update (status nil) :metrics
                                        select-keys ["n1" "n2" "n3"]))
                    cluster/voter-configs (fn [_test _status] [(set nodes)])]
        (is (= :no-snapshot-lag-target
               (get-in (nemesis/invoke! nemesis test-config
                                        {:type :info
                                         :f :start-snapshot-lag
                                         :value :isolate})
                       [:value :reason]))
            "with n4 and n5 down, losing n2 or n3 leaves no quorum of five")))

    (testing "two voters cannot lose one and keep a quorum"
      (with-redefs [cluster/membership-status (constantly (status nil))
                    cluster/voter-configs (fn [_test _status] [#{"n1" "n2"}])]
        (is (= :no-snapshot-lag-target
               (get-in (nemesis/invoke! nemesis test-config
                                        {:type :info
                                         :f :start-snapshot-lag
                                         :value :isolate})
                       [:value :reason])))))))

(defn- lag-op [f value]
  {:type :info :process :nemesis :f f :value value})

(defn- episode [mode victim & {:as details}]
  (merge {:status :installed
          :mode mode
          :leader "n1"
          :victim victim
          :purged-past-victim? true
          :purged-index 60
          :caught-up? true
          :installs-before 0
          :installs-after 1}
         details))

(defn- check [history]
  (checker/check (:checker (snapshot/snapshot-package :database))
                 test-config
                 history
                 {}))

(deftest requires-an-install-on-a-victim-for-each-mode
  (let [recovered (lag-op :await-recovery {:status :installed :leader "n1"})
        history [(lag-op :start-snapshot-lag :isolate)
                 (lag-op :start-snapshot-lag (episode :isolate "n2"))
                 (lag-op :stop-snapshot-lag nil)
                 (lag-op :stop-snapshot-lag (episode :isolate "n2"))
                 (lag-op :start-snapshot-lag :kill)
                 (lag-op :start-snapshot-lag (episode :kill "n3"))
                 (lag-op :stop-snapshot-lag nil)
                 (lag-op :stop-snapshot-lag (episode :kill "n3"))
                 recovered]]
    (testing "an install in each episode covers both modes"
      (let [result (check history)]
        (is (true? (:valid? result)))
        (is (= [:isolate :kill] (:observed-modes result)))
        (is (= 2 (:episodes-with-install result)))))

    (testing "an episode whose victim installed nothing does not count"
      (let [result (check (assoc history 7 (lag-op :stop-snapshot-lag
                                                   (episode :kill "n3"
                                                            :installs-before 1
                                                            :installs-after 1))))]
        (is (false? (:valid? result)))
        (is (= [:kill] (:missing-modes result)))))

    (testing "an episode in which the leader never purged past the victim does not count"
      (let [history (assoc history 7 (lag-op :stop-snapshot-lag
                                             (episode :kill "n3"
                                                      :purged-past-victim? false
                                                      :purged-index nil)))
            result (check history)]
        (is (= [:kill] (:missing-modes result)))))

    (testing "a victim that never catches up fails the run"
      (let [history (assoc history 7 (lag-op :stop-snapshot-lag
                                             (episode :kill "n3"
                                                      :caught-up? false)))
            result (check history)]
        (is (false? (:valid? result)))
        (is (= 1 (count (:stalled-episodes result))))))

    (testing "an open episode leaves recovery pending"
      (let [result (check (subvec history 0 6))]
        (is (= :recovery-pending (:cluster-state result)))
        (is (false? (:valid? result)))))))
