(ns jepsen.openraft.nemesis.membership-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
                    [nemesis :as nemesis]
                    [util :as util]]
            [jepsen.openraft [client :as client]
                             [cluster :as cluster]
                             [db :as openraft-db]]
            [jepsen.openraft.nemesis.membership :as membership]
            [jepsen.random :as random]))

(def nodes ["n1" "n2" "n3" "n4" "n5"])

(def test-config
  {:nodes nodes
   :api-port 21001
   :raft-port 22001})

(defn- forward-error [endpoint]
  (ex-info "forward"
           {:kind :openraft-error
            :error {:ForwardToLeader
                     {:leader_id "n2"
                      :leader_node {:data endpoint}}}}))

(deftest shrink-confirms-membership-before-wiping
  (let [calls (atom [])
        before #{"n1" "n2" "n3" "n4" "n5"}
        after #{"n1" "n2" "n3" "n4"}
        initial {:leader "n1"
                 :voters before
                 :learners #{}
                 :stable? true
                 :metrics (zipmap nodes (repeat {}))}
        final {:leader "n1"
               :voters after}
        subject (membership/->MembershipNemesis :database)]
    (with-redefs [cluster/await-committed-membership!
                  (fn [_test]
                    (swap! calls conj [:await nil])
                    initial)
                  cluster/await-stable-membership!
                  (fn [_test expected]
                    (swap! calls conj [:await expected])
                    final)
                  random/shuffle reverse
                  client/change-membership!
                  (fn [endpoint node-ids]
                    (swap! calls conj
                           [:change-membership endpoint node-ids]))
                  openraft-db/stop-and-wipe-node!
                  (fn [database _test node]
                    (swap! calls conj [:stop-and-wipe database node]))]
      (let [result (nemesis/invoke!
                     subject
                     test-config
                     {:type :info
                      :f :shrink})]
        (is (= [[:await nil]
                [:change-membership "n1:21001"
                 ["n1" "n2" "n3" "n4"]]
                [:await after]
                [:stop-and-wipe :database "n5"]]
               @calls))
        (is (= {:node "n5"
                :leader "n1"
                :before before
                :after after}
               (:value result)))))))

(deftest grow-starts-empty-node-before-adding-it
  (let [calls (atom [])
        before #{"n1" "n2" "n3" "n4"}
        after #{"n1" "n2" "n3" "n4" "n5"}
        initial {:leader "n1"
                 :voters before
                 :learners #{}
                 :stable? true
                 :non-members #{"n5"}}
        final {:leader "n1"
               :voters after}
        subject (membership/->MembershipNemesis :database)]
    (with-redefs [cluster/await-committed-membership!
                  (fn [_test]
                    (swap! calls conj [:await nil])
                    initial)
                  cluster/await-stable-membership!
                  (fn [_test expected]
                    (swap! calls conj [:await expected])
                    final)
                  openraft-db/start-empty-node!
                  (fn [database _test node]
                    (swap! calls conj [:start-empty database node]))
                  client/add-learner!
                  (fn [endpoint node-id api-addr raft-addr]
                    (swap! calls conj
                           [:add-learner
                            endpoint
                            node-id
                            api-addr
                            raft-addr]))
                  client/change-membership!
                  (fn [endpoint node-ids]
                    (swap! calls conj
                           [:change-membership endpoint node-ids]))]
      (let [result (nemesis/invoke!
                     subject
                     test-config
                     {:type :info
                      :f :grow})]
        (is (= [[:await nil]
                [:start-empty :database "n5"]
                [:add-learner
                 "n1:21001"
                 "n5"
                 "n5:21001"
                 "n5:22001"]
                [:change-membership "n1:21001"
                 ["n1" "n2" "n3" "n4" "n5"]]
                [:await after]]
               @calls))
        (is (= {:node "n5"
                :source :non-member
                :leader "n1"
                :before before
                :after after}
               (:value result)))))))

(deftest grow-promotes-an-existing-learner
  (let [calls (atom [])
        before #{"n1" "n2" "n3" "n4"}
        after #{"n1" "n2" "n3" "n4" "n5"}
        initial {:leader "n1"
                 :voters before
                 :learners #{"n5"}
                 :non-members #{}
                 :stable? true
                 :metrics {"n5" {}}}
        final {:leader "n1"
               :voters after}
        subject (membership/->MembershipNemesis :database)]
    (with-redefs [cluster/await-committed-membership! (fn [_test] initial)
                  cluster/await-stable-membership!
                  (fn [_test _expected] final)
                  openraft-db/start-empty-node!
                  (fn [& _]
                    (throw (ex-info "MUST NOT reset a learner" {})))
                  client/add-learner!
                  (fn [& _]
                    (swap! calls conj :add-learner))
                  client/change-membership!
                  (fn [& _]
                    (swap! calls conj :change-membership))]
      (nemesis/invoke! subject
                       test-config
                       {:type :info
                        :f :grow})
      (is (= [:add-learner :change-membership] @calls)))))

(deftest grow-follows-a-new-leader
  (let [calls (atom [])
        before #{"n1" "n2" "n3" "n4"}
        after #{"n1" "n2" "n3" "n4" "n5"}
        initial {:leader "n1"
                 :voters before
                 :learners #{}
                 :stable? true
                 :non-members #{"n5"}}
        final {:leader "n2"
               :voters after}
        subject (membership/->MembershipNemesis :database)]
    (with-redefs [cluster/await-committed-membership! (fn [_test] initial)
                  cluster/await-stable-membership!
                  (fn [_test _expected] final)
                  openraft-db/start-empty-node! (fn [& _])
                  client/add-learner!
                  (fn [endpoint & _]
                    (swap! calls conj [:add-learner endpoint])
                    (when (= "n1:21001" endpoint)
                      (throw (forward-error "n2:21001"))))
                  client/change-membership!
                  (fn [endpoint _node-ids]
                    (swap! calls conj [:change-membership endpoint]))]
      (nemesis/invoke! subject
                       test-config
                       {:type :info
                        :f :grow})
      (is (= [[:add-learner "n1:21001"]
              [:add-learner "n2:21001"]
              [:change-membership "n2:21001"]]
             @calls)))))

(deftest ambiguous-add-learner-uses-observed-membership
  (let [calls (atom [])
        before #{"n1" "n2" "n3" "n4"}
        after (conj before "n5")
        initial {:leader "n1"
                 :voters before
                 :learners #{}
                 :non-members #{"n5"}
                 :stable? true}
        learner-added {:leader "n1"
                       :voters before
                       :learners #{"n5"}
                       :effective-log-id {:index 7}
                       :stable? true}
        final {:leader "n1"
               :voters after}
        subject (membership/->MembershipNemesis :database)]
    (with-redefs [cluster/await-committed-membership! (fn [_test] initial)
                  cluster/membership-status
                  (fn [_test]
                    (swap! calls conj :observe-learner)
                    learner-added)
                  cluster/await-stable-membership!
                  (fn [_test expected]
                    (swap! calls conj [:await expected])
                    final)
                  openraft-db/start-empty-node! (fn [& _])
                  client/add-learner!
                  (fn [& _]
                    (swap! calls conj :add-learner)
                    (throw (ex-info "timeout"
                                    {:kind :request-timeout})))
                  client/change-membership!
                  (fn [_endpoint _node-ids]
                    (swap! calls conj :change-membership))]
      (let [result (nemesis/invoke!
                     subject
                     test-config
                     {:type :info
                      :f :grow})]
        (is (= [:add-learner
                :observe-learner
                :change-membership
                [:await after]]
               @calls))
        (is (= after (get-in result [:value :after])))))))

(deftest completes-an-existing-joint-membership
  (let [before #{"n1" "n2" "n3"}
        target #{"n1" "n2" "n3" "n4"}
        joint {:leader "n1"
               :stable? false
               :effective-voter-configs [before target]}
        stable {:leader "n1"
                :voters target
                :stable? true}
        calls (atom [])]
    (with-redefs-fn
      {#'cluster/await-committed-membership! (fn [_test] joint)
       #'client/change-membership!
       (fn [endpoint node-ids]
         (swap! calls conj [:change-membership endpoint node-ids]))
       #'cluster/await-stable-membership!
       (fn [_test expected]
         (swap! calls conj [:await expected])
         stable)}
      (fn []
        (is (= stable
               (#'membership/stable-membership! test-config)))
        (is (= [[:change-membership
                 "n1:21001"
                 ["n1" "n2" "n3" "n4"]]
                [:await target]]
               @calls))))))

(deftest retries-after-an-in-progress-membership-change
  (let [before #{"n1" "n2" "n3" "n4"}
        target (conj before "n5")
        calls (atom [])
        final {:leader "n1"
               :voters target}]
    (with-redefs-fn
      {#'client/change-membership!
       (fn [_endpoint _node-ids]
         (swap! calls conj :change-membership)
         (when (= 1 (count @calls))
           (throw (ex-info
                    "in progress"
                    {:kind :openraft-error
                     :error {:ChangeMembershipError
                             {:InProgress {}}}}))))
       #'membership/stable-membership!
       (fn [_test]
         (swap! calls conj :complete-pending)
         {:voters before})
       #'cluster/await-stable-membership!
       (fn [_test expected]
         (swap! calls conj [:await expected])
         final)}
      (fn []
        (is (= final
               (#'membership/change-membership-and-await!
                 test-config
                 (atom "n1:21001")
                 target)))
        (is (= [:change-membership
                :complete-pending
                :change-membership
                [:await target]]
               @calls))))))

(deftest rejects-an-unreachable-learner
  (let [initial {:leader "n1"
                 :voters #{"n1" "n2" "n3" "n4"}
                 :learners #{"n5"}
                 :non-members #{}
                 :stable? true
                 :metrics {"n1" {}
                           "n2" {}
                           "n3" {}
                           "n4" {}}}
        subject (membership/->MembershipNemesis :database)]
    (with-redefs [cluster/await-committed-membership! (fn [_test] initial)
                  util/await-fn (fn [f _opts] (f))
                  client/metrics!
                  (fn [_endpoint]
                    (throw (ex-info "unreachable"
                                    {:kind :unreachable})))
                  openraft-db/start-empty-node!
                  (fn [& _]
                    (throw (ex-info "MUST NOT restart a learner" {})))
                  client/add-learner!
                  (fn [& _]
                    (throw (ex-info "MUST NOT add an unreachable learner" {})))]
      (let [error (try
                    (nemesis/invoke! subject
                                     test-config
                                     {:type :info
                                      :f :grow})
                    nil
                    (catch Exception e
                      e))]
        (is (= :learner-unreachable (:kind (ex-data error))))
        (is (= "n5" (:node (ex-data error))))))))

(deftest ambiguous-shrink-completes-joint-before-cleanup
  (let [calls (atom [])
        before #{"n1" "n2" "n3" "n4" "n5"}
        after #{"n1" "n2" "n3" "n4"}
        initial {:leader "n1"
                 :voters before
                 :learners #{}
                 :stable? true
                 :metrics (zipmap nodes (repeat {}))}
        joint {:leader "n1"
               :voters before
               :stable? false
               :effective-voter-configs [before after]}
        final {:leader "n1"
               :voters after
               :stable? true}
        committed-statuses (atom [initial joint])
        stable-attempts (atom 0)
        change-attempts (atom 0)
        subject (membership/->MembershipNemesis :database)]
    (with-redefs [cluster/await-committed-membership!
                  (fn [_test]
                    (let [status (first @committed-statuses)]
                      (swap! committed-statuses subvec 1)
                      status))
                  cluster/await-stable-membership!
                  (fn [_test expected]
                    (swap! calls conj [:await expected])
                    (if (= 1 (swap! stable-attempts inc))
                      (throw (ex-info "not stable yet"
                                      {:type :timeout}))
                      final))
                  random/shuffle reverse
                  client/change-membership!
                  (fn [endpoint node-ids]
                    (swap! calls conj
                           [:change-membership endpoint node-ids])
                    (when (= 1 (swap! change-attempts inc))
                      (throw (ex-info "timeout"
                                      {:kind :request-timeout}))))
                  openraft-db/stop-and-wipe-node!
                  (fn [_database _test node]
                    (swap! calls conj [:stop-and-wipe node]))]
      (nemesis/invoke! subject
                       test-config
                       {:type :info
                        :f :shrink})
      (is (= [[:change-membership
               "n1:21001"
               ["n1" "n2" "n3" "n4"]]
              [:await after]
              [:change-membership
               "n1:21001"
               ["n1" "n2" "n3" "n4"]]
              [:await after]
              [:stop-and-wipe "n5"]]
             @calls)))))

(deftest ambiguous-shrink-does-not-clean-up-without-confirmation
  (let [calls (atom [])
        wiped? (atom false)
        before #{"n1" "n2" "n3" "n4" "n5"}
        after #{"n1" "n2" "n3" "n4"}
        initial {:leader "n1"
                 :voters before
                 :learners #{}
                 :stable? true
                 :metrics (zipmap nodes (repeat {}))}
        subject (membership/->MembershipNemesis :database)]
    (with-redefs [cluster/await-committed-membership! (fn [_test] initial)
                  cluster/await-stable-membership!
                  (fn [_test expected]
                    (swap! calls conj [:await expected])
                    (throw (ex-info "not committed" {:type :timeout})))
                  random/shuffle reverse
                  client/change-membership!
                  (fn [endpoint node-ids]
                    (swap! calls conj
                           [:change-membership endpoint node-ids])
                    (throw (ex-info "timeout"
                                    {:kind :request-timeout})))
                  openraft-db/stop-and-wipe-node!
                  (fn [& _]
                    (reset! wiped? true))]
      (is (thrown? Exception
                   (nemesis/invoke!
                     subject
                     test-config
                     {:type :info
                      :f :shrink})))
      (is (= [[:change-membership
               "n1:21001"
               ["n1" "n2" "n3" "n4"]]
              [:await after]]
             @calls))
      (is (false? @wiped?)))))

(deftest shrink-preserves-the-minimum-membership
  (let [status {:leader "n1"
                :voters #{"n1" "n2" "n3"}
                :learners #{}
                :stable? true
                :metrics {"n1" {}
                          "n2" {}
                          "n3" {}}}
        subject (membership/->MembershipNemesis :database)]
    (with-redefs [cluster/await-committed-membership!
                  (fn [_test] status)
                  client/change-membership!
                  (fn [& _]
                    (throw (ex-info "MUST NOT change membership" {})))
                  openraft-db/stop-and-wipe-node!
                  (fn [& _]
                    (throw (ex-info "MUST NOT wipe a voter" {})))]
      (let [result (nemesis/invoke!
                     subject
                     test-config
                     {:type :info
                      :f :shrink})]
        (is (= :minimum-membership (:value result)))))))

(deftest restores-all-test-nodes-as-voters
  (let [partial {:leader "n1"
                 :voters #{"n1" "n2" "n3"}}
        growing {:leader "n1"
                 :voters #{"n1" "n2" "n3" "n4"}}
        restored {:leader "n1"
                  :voters (set nodes)}
        statuses (atom [partial growing restored])
        grow-count (atom 0)
        subject (membership/->MembershipNemesis :database)]
    (with-redefs-fn
      {#'membership/stable-membership!
       (fn [_test]
         (let [status (first @statuses)]
           (swap! statuses subvec 1)
           status))
       #'membership/grow!
       (fn [_database _test]
         (swap! grow-count inc))
       #'cluster/await-ready!
       (fn [_test]
         {:leader "n2"})}
      (fn []
        (let [result (nemesis/invoke!
                       subject
                       test-config
                       {:type :info
                        :f :restore-membership})]
          (is (= 2 @grow-count))
          (is (= {:leader "n2"
                  :voters (set nodes)}
                 (:value result))))))))

(deftest restore-fails-when-grow-cannot-progress
  (let [partial {:leader "n1"
                 :voters #{"n1" "n2" "n3"}}
        subject (membership/->MembershipNemesis :database)]
    (with-redefs-fn
      {#'membership/stable-membership! (fn [_test] partial)
       #'membership/grow! (fn [_database _test] :membership-full)}
      (fn []
        (let [error (try
                      (nemesis/invoke!
                        subject
                        test-config
                        {:type :info
                         :f :restore-membership})
                      nil
                      (catch clojure.lang.ExceptionInfo e
                        e))]
          (is (= "Unable to restore OpenRaft membership"
                 (ex-message error)))
          (is (= :membership-full
                 (:result (ex-data error)))))))))

(deftest membership-package-requires-room-to-change
  (let [three-node-test (assoc test-config
                               :nodes ["n1" "n2" "n3"])
        error (try
                (membership/membership-package :database three-node-test)
                nil
                (catch clojure.lang.ExceptionInfo e
                  e))]
    (is (= "Membership nemesis requires more than three nodes"
           (ex-message error)))
    (is (= ["n1" "n2" "n3"]
           (:nodes (ex-data error))))))

(deftest checks-membership-coverage-and-restoration
  (let [subject (:checker (membership/membership-package
                            :database
                            test-config))
        all-voters (set nodes)
        four-voters #{"n1" "n2" "n3" "n4"}
        shrink {:type :info
                :f :shrink
                :value {:before all-voters
                        :after four-voters}}
        grow {:type :info
              :f :grow
              :value {:before four-voters
                      :after all-voters}}
        restore {:type :info
                 :f :restore-membership
                 :value {:leader "n1"
                         :voters all-voters}}
        complete-history [shrink grow restore]]
    (testing "both changes ran and the full membership was restored"
      (is (:valid? (checker/check subject
                                  test-config
                                  complete-history
                                  {}))))

    (testing "a no-op does not count as membership change coverage"
      (let [result (checker/check
                     subject
                     test-config
                     [shrink
                      {:type :info
                       :f :grow
                       :value :membership-full}
                      restore]
                     {})]
        (is (false? (:valid? result)))
        (is (= [:grow] (:missing-changes result)))))

    (testing "the final membership must be restored"
      (let [result (checker/check subject
                                  test-config
                                  [shrink grow]
                                  {})]
        (is (false? (:valid? result)))
        (is (false? (:restored? result)))))

    (testing "a later change invalidates an earlier restoration"
      (let [result (checker/check subject
                                  test-config
                                  (conj complete-history shrink)
                                  {})]
        (is (false? (:valid? result)))
        (is (false? (:restored? result)))))

    (testing "a nemesis operation error invalidates the test"
      (let [result (checker/check
                     subject
                     test-config
                     (conj complete-history
                           {:type :info
                            :f :grow
                            :error "learner unavailable"
                            :exception {:kind :learner-unreachable}})
                     {})]
        (is (false? (:valid? result)))
        (is (= 1 (:error-count result)))))))
